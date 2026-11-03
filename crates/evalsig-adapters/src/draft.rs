//! `map draft` and `map decide` (SPEC §5.3): a mapping drafted from a probe, with the
//! choices that change results left as `DECIDE` for a person, and those choices
//! written back one at a time.
//!
//! The draft never fills a decision: not the item key and cluster, the value
//! semantics, the positive definition and decision rule, the missing reasons, the
//! treatment of values without a conclusion, or the replicate kind. It also leaves the
//! judgment value's path to a person, since a guess there changes every result.

use serde_json::{Value as Json, json};
use toml::{Table, Value};

use crate::mapping::SCHEMA;
use crate::probe::{PathStat, Probe, Shape};

const DECIDE: &str = "DECIDE";

/// A drafted mapping and what a person must decide.
#[derive(Debug, Clone)]
pub struct Draft {
    pub mapping: Table,
    pub decisions_needed: Vec<Json>,
}

fn table(pairs: &[(&str, Value)]) -> Value {
    Value::Table(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

fn s(v: &str) -> Value {
    Value::String(v.to_owned())
}

fn last_name(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

/// Paths that could hold the judgment value: short scalars in every row.
fn value_candidates(p: &Probe) -> Vec<&PathStat> {
    p.paths
        .iter()
        .filter(|s| !s.path.contains("[*]") && s.rows == p.rows)
        .filter(|s| !p.key_candidates.contains(&s.path))
        .filter(|s| {
            s.types
                .iter()
                .all(|t| matches!(*t, "boolean" | "integer" | "number"))
                || (s.types.iter().all(|t| *t == "string") && s.values.is_some())
        })
        .collect()
}

/// Draft a mapping for records of this shape.
pub fn draft(name: &str, p: &Probe) -> Draft {
    let mut decisions = Vec::new();
    let format = match p.shape {
        Shape::Jsonl => "jsonl",
        Shape::Csv => "csv",
        Shape::Json | Shape::Zip => "json",
    };
    let mut source = vec![("format", s(format))];
    if p.shape == Shape::Json
        && p.rows == 1
        && let Some(rp) = p.records_path_candidates.first()
    {
        source.push(("records_path", s(rp)));
    }
    let mut m = Table::new();
    m.insert("schema".into(), s(SCHEMA));
    m.insert("name".into(), s(name));
    m.insert("source".into(), table(&source));
    m.insert(
        "run".into(),
        table(&[
            ("run_id", table(&[("arg", s("run-id"))])),
            (
                "system",
                table(&[
                    ("id", table(&[("const", s(name))])),
                    (
                        "version",
                        table(&[("arg", s("system-version")), ("default", s("unknown"))]),
                    ),
                ]),
            ),
            ("value_semantics", s(DECIDE)),
            ("output_type", s(DECIDE)),
        ]),
    );
    m.insert("item".into(), table(&[("id", s(DECIDE))]));
    decisions.push(json!({
        "id": "item_key",
        "question": "What is one item, and which items are correlated (the cluster)?",
        "options": p.key_candidates,
        "how": "evalsig map decide item_key '<path or template> [cluster=<path>]' --by <who>",
        "effect": "The number of items and the width of every interval",
    }));
    decisions.push(json!({
        "id": "value_semantics",
        "question": "Is the value a system decision compared with reference labels, or a grade of the output?",
        "options": {"decision": "hit rate, false-alarm rate and AUC against labels", "score": "mean score without labels"},
        "effect": "Which metrics exist (evalsig capabilities)",
    }));
    let mut judgment = Table::new();
    judgment.insert("value".into(), s(DECIDE));
    decisions.push(json!({
        "id": "value",
        "question": "Which path holds the judgment value?",
        "options": value_candidates(p).iter().map(|c| json!({"path": c.path, "types": c.types, "values": c.values})).collect::<Vec<_>>(),
        "effect": "Every result",
    }));
    decisions.push(json!({
        "id": "output_type",
        "question": "Is the value binary, ordinal with how many levels, or a continuous score?",
        "options": ["binary", "ordinal:<levels>", "score"],
        "effect": "Which metrics and intervals apply",
    }));
    decisions.push(json!({
        "id": "value.none",
        "question": "How do values without a conclusion or with partial credit map (for example none or P)?",
        "options": {"value_map": "map each listed value; values left out become missing (invalid_output)"},
        "how": "evalsig map decide value.none '{\"low\": 0, \"none\": 0}' --by <who>",
        "effect": "The rows with such values move between the lowest level and missing",
    }));
    if let Some(r) = p.replicate_candidates.first() {
        judgment.insert("gen_replicate".into(), s(DECIDE));
        decisions.push(json!({
            "id": "replicate_kind",
            "question": format!("Is {r} a re-run of the system or a re-grading of the same output?"),
            "options": {format!("gen_replicate={r}"): "generation replicate", format!("grade_replicate={r}"): "grading replicate"},
            "effect": "Whether replicate disagreement measures the system or the grader",
        }));
    }
    let missing: Vec<&PathStat> = p
        .paths
        .iter()
        .filter(|s| !s.path.contains("[*]"))
        .filter(|s| {
            let n = last_name(&s.path).to_ascii_lowercase();
            ["error", "missing", "fail"].iter().any(|w| n.contains(w))
        })
        .collect();
    if let Some(mp) = missing.first() {
        judgment.insert(
            "missing_if".into(),
            Value::Array(vec![table(&[
                (
                    "when",
                    table(&[("path", s(&mp.path)), ("exists", Value::Boolean(true))]),
                ),
                ("reason", s(DECIDE)),
            ])]),
        );
        decisions.push(json!({
            "id": "missing.reason",
            "question": format!("Which missing reason does each value of {} become: system side or measurement side?", mp.path),
            "options": {"system side": ["error", "timeout", "refused", "invalid_output", "limit"], "measurement side": ["grader_failed", "not_run", "invalidated", "excluded", "other"]},
            "values": mp.values,
            "how": "evalsig map decide missing.reason '{\"provider_error\": \"error\"}' --by <who>",
            "effect": "System-side missing values count as failures in gates; measurement-side ones leave the denominator",
        }));
    }
    m.insert("judgment".into(), Value::Table(judgment));
    decisions.push(json!({
        "id": "positive",
        "question": "Which reference label is positive, and from which judgment value is a decision positive?",
        "options": {"positive": [{"equals": true}], "decision": [{"ordinal_at_least": 2}]},
        "applies_to": "config",
        "effect": "Hit and false-alarm rates; ROC and AUC do not change",
    }));
    Draft {
        mapping: m,
        decisions_needed: decisions,
    }
}

/// The decision IDs `decide` accepts.
pub const DECISIONS: [&str; 8] = [
    "item_key",
    "value_semantics",
    "value",
    "output_type",
    "value.none",
    "replicate_kind",
    "missing.reason",
    "positive",
];

fn get_table<'a>(m: &'a mut Table, key: &str) -> Result<&'a mut Table, String> {
    m.get_mut(key)
        .and_then(Value::as_table_mut)
        .ok_or_else(|| format!("the mapping has no [{key}]"))
}

fn json_choice(choice: &str) -> Result<Table, String> {
    let v: Json =
        serde_json::from_str(choice).map_err(|e| format!("expected a JSON object: {e}"))?;
    let t: Value = serde_json::from_value(v).map_err(|e| e.to_string())?;
    match t {
        Value::Table(t) => Ok(t),
        _ => Err("expected a JSON object".into()),
    }
}

/// Write one decision into the mapping and record it under `[[decisions]]`.
pub fn decide(
    m: &mut Table,
    id: &str,
    choice: &str,
    by: &str,
    ledger_seq: u64,
) -> Result<(), String> {
    let applies_to = match id {
        "item_key" => {
            let (expr, cluster) = match choice.split_once(" cluster=") {
                Some((e, c)) => (e.trim(), Some(c.trim())),
                None => (choice.trim(), None),
            };
            let item = get_table(m, "item")?;
            let value = if expr.starts_with(['$', '@']) {
                s(expr)
            } else {
                table(&[("template", s(expr))])
            };
            item.insert("id".into(), value);
            if let Some(c) = cluster {
                item.insert("cluster".into(), s(c));
            }
            "mapping"
        }
        "value_semantics" => {
            if !matches!(choice, "decision" | "score") {
                return Err("value_semantics is decision or score".into());
            }
            get_table(m, "run")?.insert("value_semantics".into(), s(choice));
            "mapping"
        }
        "output_type" => {
            let (kind, levels) = match choice.split_once(':') {
                Some((k, n)) => (
                    k,
                    Some(n.parse::<i64>().map_err(|_| "levels must be a number")?),
                ),
                None => (choice, None),
            };
            if !matches!(kind, "binary" | "ordinal" | "score")
                || (kind == "ordinal") != levels.is_some()
            {
                return Err("output_type is binary, ordinal:<levels> or score".into());
            }
            let run = get_table(m, "run")?;
            run.insert("output_type".into(), s(kind));
            if let Some(n) = levels {
                run.insert("levels".into(), Value::Integer(n));
            }
            "mapping"
        }
        "value" => {
            if !choice.starts_with('$') {
                return Err("value is a path such as $.verdict".into());
            }
            get_table(m, "judgment")?.insert("value".into(), s(choice));
            "mapping"
        }
        "value.none" => {
            let map = json_choice(choice)?;
            let j = get_table(m, "judgment")?;
            let path = match j.get("value") {
                Some(Value::String(p)) if p != DECIDE => p.clone(),
                Some(Value::Table(t)) => t
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or("judgment.value has no path")?
                    .to_owned(),
                _ => return Err("decide value before value.none".into()),
            };
            j.insert(
                "value".into(),
                table(&[("path", s(&path)), ("value_map", Value::Table(map))]),
            );
            "mapping"
        }
        "replicate_kind" => {
            let (kind, path) = choice
                .split_once('=')
                .ok_or("replicate_kind is gen_replicate=<path> or grade_replicate=<path>")?;
            if !matches!(kind, "gen_replicate" | "grade_replicate") {
                return Err("replicate_kind is gen_replicate or grade_replicate".into());
            }
            let j = get_table(m, "judgment")?;
            j.remove("gen_replicate");
            j.insert(kind.into(), s(path));
            "mapping"
        }
        "missing.reason" => {
            let map = json_choice(choice)?;
            let j = get_table(m, "judgment")?;
            let rule = j
                .get_mut("missing_if")
                .and_then(Value::as_array_mut)
                .and_then(|a| a.first_mut())
                .and_then(Value::as_table_mut)
                .ok_or("the mapping has no judgment.missing_if")?;
            let path = rule
                .get("when")
                .and_then(|w| w.get("path"))
                .and_then(Value::as_str)
                .ok_or("missing_if has no when.path")?
                .to_owned();
            rule.insert(
                "reason".into(),
                table(&[
                    ("path", s(&path)),
                    ("value_map", Value::Table(map)),
                    ("default", s("other")),
                ]),
            );
            "mapping"
        }
        "positive" => {
            json_choice(choice)?;
            "config"
        }
        other => {
            return Err(format!(
                "no decision {other:?}; decisions: {}",
                DECISIONS.join(", ")
            ));
        }
    };
    let record = table(&[
        ("id", s(id)),
        ("choice", s(choice)),
        ("decided_by", s(by)),
        ("ledger_seq", Value::Integer(ledger_seq as i64)),
        ("applies_to", s(applies_to)),
    ]);
    match m.get_mut("decisions").and_then(Value::as_array_mut) {
        Some(list) => list.push(record),
        None => {
            m.insert("decisions".into(), Value::Array(vec![record]));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Limits;
    use crate::mapping;
    use crate::probe;

    const LOG: &[u8] = include_bytes!("../../../tests/fixtures/probe/secret-like.jsonl");

    fn drafted() -> Draft {
        let rows = probe::rows(Shape::Jsonl, LOG, &Limits::default()).unwrap();
        draft("support", &probe::probe(Shape::Jsonl, &rows, false))
    }

    #[test]
    fn the_draft_fills_none_of_the_decisions() {
        let d = drafted();
        let text = toml::to_string(&d.mapping).unwrap();
        let c = mapping::compile(&mapping::parse(&text).unwrap()).unwrap();
        assert_eq!(
            c.undecided,
            [
                "run.value_semantics",
                "run.output_type",
                "item.id",
                "judgment.gen_replicate",
                "judgment.value",
                "judgment.missing_if[0].reason",
            ]
        );
        let ids: Vec<&str> = d
            .decisions_needed
            .iter()
            .map(|x| x["id"].as_str().unwrap())
            .collect();
        for id in [
            "item_key",
            "value_semantics",
            "replicate_kind",
            "missing.reason",
            "value.none",
            "positive",
        ] {
            assert!(ids.contains(&id), "{id}");
        }
        assert!(!text.contains("cluster"));
    }

    #[test]
    fn decisions_fill_the_draft_one_by_one() {
        let mut m = drafted().mapping;
        for (n, (id, choice)) in [
            ("item_key", "$.ticket cluster=$.ticket"),
            ("value_semantics", "decision"),
            ("value", "$.verdict"),
            ("output_type", "binary"),
            ("value.none", r#"{"approve": true, "reject": false}"#),
            ("replicate_kind", "gen_replicate=$.attempt"),
            ("missing.reason", r#"{"timeout": "timeout"}"#),
            ("positive", r#"{"positive": {"equals": true}}"#),
        ]
        .into_iter()
        .enumerate()
        {
            decide(&mut m, id, choice, "user:alice", n as u64).unwrap();
        }
        let text = toml::to_string(&m).unwrap();
        let c = mapping::compile(&mapping::parse(&text).unwrap()).unwrap();
        assert!(c.undecided.is_empty(), "{:?}", c.undecided);
        assert_eq!(m["decisions"].as_array().unwrap().len(), 8);
        assert!(decide(&mut m, "nope", "x", "user:alice", 9).is_err());
        assert!(decide(&mut m, "value_semantics", "maybe", "user:alice", 9).is_err());
    }
}
