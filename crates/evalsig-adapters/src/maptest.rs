//! `map test` (SPEC §5.3): how much a mapping loses. Coverage of the source rows, paths
//! the mapping never reads, key integrity, the distribution of mapped values, and the
//! expectations of `mapping.tests.jsonl`.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value as Json;

use crate::mapping::{self, Compiled, Host, Issue, MapError};
use crate::probe::{self, Shape};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub rows: usize,
    /// Rows that gave records without a problem.
    pub mapped: usize,
    /// Rows left out, by the field that failed.
    pub dropped: usize,
    pub dropped_by: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Integrity {
    /// Judgments beyond the first with the same (item, run, task, replicates).
    pub duplicate_judgments: usize,
    /// Item IDs mapped to different item records.
    pub conflicting_items: usize,
    /// (item, run, task) groups whose generation replicates are not 0, 1, …, n-1.
    pub replicate_gaps: usize,
    /// Items without a label, when the mapping maps labels.
    pub items_without_label: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Values {
    /// Judgment values and how often each occurs.
    pub judgments: BTreeMap<String, usize>,
    /// Missing judgments by reason.
    pub missing: BTreeMap<String, usize>,
    /// Source values `value_map` did not cover, with counts.
    pub not_mapped: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseFailure {
    pub line: usize,
    pub expected: Vec<Json>,
    pub got: Vec<Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Cases {
    pub cases: usize,
    pub passed: usize,
    pub failures: Vec<CaseFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub coverage: Coverage,
    pub unused_paths: Vec<String>,
    pub integrity: Integrity,
    pub values: Values,
}

/// Path names of a probed leaf: `$.a.b[*].c` is `a`, `b`, `c`.
fn probed_names(path: &str) -> Vec<&str> {
    path.trim_start_matches('$')
        .split('.')
        .map(|s| s.trim_end_matches("[*]"))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Leaf paths of the source that the mapping never reads.
fn unused(rows: &[(usize, Json)], compiled: &Compiled) -> Vec<String> {
    let read = compiled.read_paths();
    let rooted: Vec<Vec<Option<String>>> = read
        .iter()
        .filter(|p| p.from_row())
        .map(|p| p.names())
        .collect();
    // Paths from the current element (features, filters) are matched by their last name.
    let relative: BTreeSet<String> = read
        .iter()
        .filter(|p| !p.from_row())
        .filter_map(|p| p.names().last().cloned().flatten())
        .collect();
    let probed = probe::probe(Shape::Jsonl, rows.iter().map(|(_, r)| r), true);
    probed
        .paths
        .into_iter()
        .filter(|s| !s.types.iter().any(|t| *t == "object" || *t == "array"))
        .map(|s| s.path)
        .filter(|p| {
            let names = probed_names(p);
            let covered = rooted.iter().any(|pat| {
                pat.len() <= names.len()
                    && pat
                        .iter()
                        .zip(&names)
                        .all(|(want, got)| want.as_deref().is_none_or(|w| w == *got))
            });
            !covered && !names.last().is_some_and(|l| relative.contains(*l))
        })
        .collect()
}

/// A judgment's primary key: item, run, task and the two replicates.
type JudgmentKey<'r> = (&'r Json, &'r Json, String, Option<u64>, Option<u64>);

/// More distinct values than this are summarised under `OTHERS`.
const MAX_VALUES: usize = 20;
const OTHERS: &str = "(other values)";

/// Count a value; long or secret-like text is counted without being shown, as in
/// `map probe`.
fn count(map: &mut BTreeMap<String, usize>, value: &str) {
    let shown = if value.chars().count() > crate::probe::MAX_LISTED_LEN
        || evalsig_core::validity::secret_shape(value).is_some()
    {
        "(withheld text)"
    } else if map.len() >= MAX_VALUES && !map.contains_key(value) {
        OTHERS
    } else {
        value
    };
    *map.entry(shown.to_owned()).or_insert(0) += 1;
}

fn text(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The report of mapping these rows.
pub fn report(
    rows: &[(usize, Json)],
    records: &[(usize, Json)],
    issues: &[Issue],
    compiled: &Compiled,
) -> Report {
    let failed: BTreeSet<usize> = issues.iter().map(|i| i.row).collect();
    let mut dropped_by = BTreeMap::new();
    for i in issues {
        *dropped_by.entry(i.at.clone()).or_insert(0) += 1;
    }
    let coverage = Coverage {
        rows: rows.len(),
        mapped: rows.iter().filter(|(n, _)| !failed.contains(n)).count(),
        dropped: failed.len(),
        dropped_by,
    };
    let mut integrity = Integrity::default();
    let mut values = Values::default();
    let mut keys: std::collections::HashSet<JudgmentKey<'_>> = std::collections::HashSet::new();
    let mut items: BTreeMap<String, &Json> = BTreeMap::new();
    let mut labelled = BTreeSet::new();
    let mut replicates: BTreeMap<(String, String, String), BTreeSet<u64>> = BTreeMap::new();
    for (_, r) in records {
        match r["record"].as_str() {
            Some("item") => {
                let id = text(&r["item_id"]);
                match items.get(&id) {
                    Some(prev) if *prev != r => integrity.conflicting_items += 1,
                    Some(_) => {}
                    None => {
                        items.insert(id, r);
                    }
                }
            }
            Some("label") => {
                labelled.insert(text(&r["item_id"]));
            }
            Some("judgment") => {
                let task = r["task"].as_str().unwrap_or("default").to_owned();
                let key = (
                    &r["item_id"],
                    &r["run_id"],
                    task.clone(),
                    r["gen_replicate"].as_u64(),
                    r["grade_replicate"].as_u64(),
                );
                if !keys.insert(key) {
                    integrity.duplicate_judgments += 1;
                }
                if let Some(g) = r["gen_replicate"].as_u64() {
                    replicates
                        .entry((text(&r["item_id"]), text(&r["run_id"]), task))
                        .or_default()
                        .insert(g);
                }
                match (&r["value"], &r["missing"]) {
                    (v, _) if !v.is_null() => count(&mut values.judgments, &text(v)),
                    (_, m) => {
                        *values.missing.entry(text(&m["reason"])).or_insert(0) += 1;
                        if let Some(raw) = m["detail"]
                            .as_str()
                            .and_then(|d| d.strip_suffix(" is not mapped"))
                            .and_then(|d| {
                                d.split_once(": ")
                                    .map(|(_, v)| v.trim_matches('"').to_owned())
                            })
                        {
                            count(&mut values.not_mapped, &raw);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    integrity.replicate_gaps = replicates
        .values()
        .filter(|g| g.last().is_some_and(|max| *max + 1 != g.len() as u64))
        .count();
    if compiled.maps_labels() {
        integrity.items_without_label =
            Some(items.keys().filter(|i| !labelled.contains(*i)).count());
    }
    Report {
        coverage,
        unused_paths: unused(rows, compiled),
        integrity,
        values,
    }
}

fn sorted(mut v: Vec<Json>) -> Vec<Json> {
    v.sort_by_key(|x| x.to_string());
    v
}

/// Check `mapping.tests.jsonl` lines `{"input": <row>, "records": [...]}`: the mapping
/// must give exactly these records for the row.
pub fn cases(c: &Compiled, tests: &str, host: &mut dyn Host) -> Result<Cases, MapError> {
    let mut out = Cases::default();
    for (i, line) in tests
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let bad = |m: &str| MapError::Read(format!("tests line {}: {m}", i + 1));
        let case: Json = serde_json::from_str(line).map_err(|e| bad(&e.to_string()))?;
        let expected = sorted(
            case["records"]
                .as_array()
                .cloned()
                .ok_or_else(|| bad("needs \"input\" and \"records\""))?,
        );
        let (got, issues) = mapping::apply(c, &[(1, case["input"].clone())], host)?;
        let got = sorted(got.into_iter().map(|(_, r)| r).collect());
        out.cases += 1;
        // A row the mapping cannot map fails its case, whatever records it gave.
        if issues.is_empty() && got == expected {
            out.passed += 1;
        } else {
            out.failures.push(CaseFailure {
                line: i + 1,
                expected,
                got,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_paths_cover_templates_insertions_and_filters() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/mapping.toml"
        ))
        .unwrap();
        let c = crate::mapping::compile(&crate::mapping::parse(&text).unwrap()).unwrap();
        let rows: Vec<(usize, Json)> = vec![(
            1,
            serde_json::json!({"schema": "triage-run/1", "task": "t", "ticket_id": "x",
                "runs": {"a": {"level": "low", "rules": [{"rule_id": "r", "applies": true, "fired": false}]}},
                "extra": {"unused": 1}}),
        )];
        assert_eq!(unused(&rows, &c), ["$.extra.unused"]);
    }
}
