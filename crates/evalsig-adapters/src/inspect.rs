//! The Inspect adapter (SPEC §5.2 ③): `.eval` logs (zip archives of JSON entries,
//! Zstandard or Deflate) and `.json` logs, `EvalLog.version` 2.
//!
//! - One run per log: `eval.eval_id`, with `system.version` built from
//!   `task@task_version`, `model` and `revision.commit` (+dirty), and the scorer as
//!   `grader`.
//! - One judgment per sample and epoch: `gen_replicate` is `epoch - 1`. C/I/P are
//!   grades of the output, so the run's value semantics are `score` and no reference
//!   label is synthesized; the target becomes `reference_ref` content.
//! - Unscored values (NaN), errors, limits and invalidations become `missing` with
//!   their reason; values that cannot be mapped become `invalid_output`, never 0.

use std::collections::BTreeMap;

use serde_json::{Map, Value as Json, json};

use crate::BodySink;
use crate::limits::{self, LimitError, Limits};
use crate::mapping::path::text;
use crate::source::scalar_fields;

/// The `EvalLog.version` values this adapter reads.
pub const SUPPORTED_VERSIONS: [u64; 1] = [2];

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error(transparent)]
    Limit(#[from] LimitError),
    #[error("{entry}: not JSON: {message}")]
    Json { entry: String, message: String },
    #[error("unsupported Inspect log version {0:?}; this evalsig reads version 2")]
    Version(Json),
    #[error("the log has no header.json: the evaluation did not finish")]
    Unfinished,
    #[error("samples have several scores ({0}); choose one with `score` in the [[import]] block")]
    WhichScore(String),
    #[error("no sample has a score: the evaluation was not scored")]
    NoScores,
    #[error("the log's status is {0}; import a log whose evaluation finished (status success)")]
    Status(String),
    #[error("sample {sample}: {message}")]
    Sample { sample: String, message: String },
}

/// How the scores of a log map to judgments.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Which entry of `samples[].scores` becomes the judgment.
    pub score: Option<String>,
    /// Score value to judgment value; `C`, `I` and `P` map to 1, 0 and 0.5 unless
    /// given here.
    pub value_map: BTreeMap<String, Json>,
    /// `score` unless the block says otherwise.
    pub value_semantics: Option<String>,
}

/// JSON with Python's non-standard `NaN`, `Infinity` and `-Infinity` read as `null`,
/// which is how the adapter sees an unscored value.
fn parse_lenient(entry: &str, bytes: &[u8]) -> Result<Json, InspectError> {
    // Most logs have no such constants; only a failed strict parse pays for the rewrite.
    if let Ok(v) = serde_json::from_slice(bytes) {
        return Ok(v);
    }
    let mut out = Vec::with_capacity(bytes.len());
    let (mut in_string, mut escaped, mut i) = (false, false, 0);
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            out.push(b);
            i += 1;
            continue;
        }
        let rest = &bytes[i..];
        let constant = [&b"-Infinity"[..], b"Infinity", b"NaN"]
            .into_iter()
            .find(|c| rest.starts_with(c));
        match (b, constant) {
            (b'"', _) => {
                in_string = true;
                out.push(b);
                i += 1;
            }
            (_, Some(c)) => {
                out.extend_from_slice(b"null");
                i += c.len();
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    serde_json::from_slice(&out).map_err(|e| InspectError::Json {
        entry: entry.to_owned(),
        message: e.to_string(),
    })
}

/// The log header and its samples, from a `.eval` archive or a `.json` log.
pub fn read_log(
    bytes: &[u8],
    zip: bool,
    limits: &Limits,
) -> Result<(Json, Vec<Json>), InspectError> {
    if !zip {
        limits::check_json_depth(bytes, limits)?;
        let mut log = parse_lenient("log", bytes)?;
        let samples = match log.get_mut("samples").map(Json::take) {
            Some(Json::Array(xs)) => xs,
            _ => Vec::new(),
        };
        return Ok((log, samples));
    }
    let mut header = None;
    let mut samples = Vec::new();
    let wanted =
        |n: &str| n == "header.json" || (n.starts_with("samples/") && n.ends_with(".json"));
    for (name, data) in limits::zip_entries(bytes, limits, wanted)? {
        limits::check_json_depth(&data, limits)?;
        if name == "header.json" {
            header = Some(parse_lenient(&name, &data)?);
        } else if name.starts_with("samples/") && name.ends_with(".json") {
            samples.push(parse_lenient(&name, &data)?);
        }
    }
    Ok((header.ok_or(InspectError::Unfinished)?, samples))
}

/// The system-independent input: the text of user messages, not the system prompt.
fn user_input(input: &Json) -> Option<Json> {
    match input {
        Json::String(_) => Some(input.clone()),
        Json::Array(messages) => {
            let user: Vec<Json> = messages
                .iter()
                .filter(|m| m["role"] == "user")
                .map(|m| m["content"].clone())
                .collect();
            (!user.is_empty()).then_some(Json::Array(user))
        }
        _ => None,
    }
}

fn system_version(spec: &Json) -> String {
    let mut v = format!(
        "{}@{} {}",
        text(&spec["task"]).unwrap_or_default(),
        text(&spec["task_version"]).unwrap_or_else(|| "0".into()),
        text(&spec["model"]).unwrap_or_default(),
    );
    if let Some(commit) = spec["revision"]["commit"].as_str() {
        v.push(' ');
        v.push_str(commit);
        if spec["revision"]["dirty"] == true {
            v.push_str("+dirty");
        }
    }
    v
}

/// Inspect's unscored reasons to evalsig's missing reasons.
fn unscored_reason(reason: &Json) -> &'static str {
    match reason.as_str() {
        Some("refusal") => "refused",
        Some("grader_failed" | "scoring_failed") => "grader_failed",
        _ => "other",
    }
}

/// The judgment value of a score, or the missing record it becomes.
fn value_of(score: &Json, options: &Options) -> Result<Json, Json> {
    let value = &score["value"];
    if value.is_null() {
        let reason = score
            .get("reason")
            .or_else(|| score["metadata"].get("unscored_reason"))
            .unwrap_or(&Json::Null);
        return Err(json!({"reason": unscored_reason(reason), "detail": "unscored"}));
    }
    let default = |k: &str| match k {
        "C" => Some(json!(1)),
        "I" => Some(json!(0)),
        "P" => Some(json!(0.5)),
        _ => None,
    };
    let mapped = match value {
        Json::String(s) => options.value_map.get(s).cloned().or_else(|| default(s)),
        Json::Bool(b) => Some(json!(u8::from(*b))),
        Json::Number(_) => Some(value.clone()),
        _ => None,
    };
    mapped.ok_or_else(|| {
        json!({"reason": "invalid_output", "detail": format!("score value {value} is not mapped")})
    })
}

/// The canonical records of one log, as record JSON for the importer to validate.
pub fn records(
    log: &Json,
    samples: &[Json],
    options: &Options,
    host: &mut dyn BodySink,
) -> Result<Vec<Json>, InspectError> {
    if !SUPPORTED_VERSIONS
        .iter()
        .any(|v| log["version"].as_u64() == Some(*v))
    {
        return Err(InspectError::Version(log["version"].clone()));
    }
    // Cancelled or failed runs hold only some samples; the rest would vanish unseen.
    match log["status"].as_str() {
        Some("success") | None => {}
        Some(other) => return Err(InspectError::Status(other.to_owned())),
    }
    let invalidated = log["invalidated"] == true;
    let spec = &log["eval"];
    let names: Vec<String> = samples
        .iter()
        .flat_map(|s| {
            s["scores"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, _)| k.clone())
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let score = match (&options.score, names.as_slice()) {
        (Some(s), _) => s.clone(),
        (None, [one]) => one.clone(),
        (None, []) => return Err(InspectError::NoScores),
        (None, _) => return Err(InspectError::WhichScore(names.join(", "))),
    };
    let scorer = log["results"]["scores"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["name"] == score.as_str())
        .and_then(|s| s["scorer"].as_str())
        .unwrap_or(&score)
        .to_owned();
    let run_id = text(&spec["eval_id"]).unwrap_or_default();
    let mut out = vec![json!({
        "record": "run",
        "run_id": run_id,
        "system": {"id": text(&spec["task"]).unwrap_or_default(), "version": system_version(spec)},
        "grader": {"id": scorer},
        "value_semantics": options.value_semantics.as_deref().unwrap_or("score"),
        "output_type": "score",
    })];
    let body = |host: &mut dyn BodySink, v: &Json, id: &str| {
        host.body(v).map_err(|message| InspectError::Sample {
            sample: id.to_owned(),
            message,
        })
    };
    for s in samples {
        // IDs may be integers or strings; 1 and "1" are the same sample.
        let id = text(&s["id"]).ok_or_else(|| InspectError::Sample {
            sample: s["id"].to_string(),
            message: "has no id".into(),
        })?;
        let epoch =
            s["epoch"]
                .as_u64()
                .filter(|e| *e >= 1)
                .ok_or_else(|| InspectError::Sample {
                    sample: id.clone(),
                    message: "has no epoch".into(),
                })?;
        let mut item = Map::new();
        item.insert("record".into(), json!("item"));
        item.insert("item_id".into(), json!(id));
        let strata = scalar_fields(&s["metadata"], &[]);
        if !strata.is_empty() {
            item.insert("strata".into(), Json::Object(strata));
        }
        if let Some(input) = user_input(&s["input"]) {
            item.insert("input_ref".into(), json!(body(host, &input, &id)?));
        }
        if !s["target"].is_null() {
            item.insert(
                "reference_ref".into(),
                json!(body(host, &s["target"], &id)?),
            );
        }
        out.push(Json::Object(item));
        let mut j = Map::new();
        j.insert("record".into(), json!("judgment"));
        j.insert("item_id".into(), json!(id));
        j.insert("run_id".into(), json!(run_id));
        j.insert("gen_replicate".into(), json!(epoch - 1));
        j.insert("output_type".into(), json!("score"));
        let missing = if let Some(e) = s.get("error").filter(|e| !e.is_null()) {
            Some(json!({"reason": "error", "detail": text(&e["message"]).unwrap_or_default()}))
        } else if let Some(l) = s.get("limit").filter(|l| !l.is_null()) {
            Some(json!({"reason": "limit", "detail": text(&l["type"]).unwrap_or_default()}))
        } else if invalidated || s.get("invalidation").is_some_and(|v| !v.is_null()) {
            Some(json!({"reason": "invalidated"}))
        } else {
            match s["scores"].get(&score) {
                None => Some(json!({"reason": "not_run", "detail": format!("no {score} score")})),
                Some(sc) => match value_of(sc, options) {
                    Ok(v) => {
                        j.insert("value".into(), v);
                        None
                    }
                    Err(m) => Some(m),
                },
            }
        };
        if let Some(m) = missing {
            j.insert("missing".into(), m);
        }
        if let Some(c) = s["output"]["completion"].as_str() {
            j.insert("output_ref".into(), json!(body(host, &json!(c), &id)?));
        }
        out.push(Json::Object(j));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVAL: &[u8] = include_bytes!("../../../tests/fixtures/harness/inspect/probe.eval");
    const JSON: &[u8] = include_bytes!("../../../tests/fixtures/harness/inspect/probe.json");

    use crate::testing::Recorder;

    fn kinds(records: &[Json]) -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for r in records {
            *m.entry(r["record"].as_str().unwrap().to_owned())
                .or_default() += 1;
        }
        m
    }

    #[test]
    fn the_fixture_logs_import_with_the_expected_counts() {
        for (bytes, zip) in [(EVAL, true), (JSON, false)] {
            let (log, samples) = read_log(bytes, zip, &Limits::default()).unwrap();
            assert_eq!(samples.len(), 4);
            let mut bodies = Recorder::default();
            let rs = records(&log, &samples, &Options::default(), &mut bodies).unwrap();
            assert_eq!(
                kinds(&rs),
                BTreeMap::from([
                    ("item".into(), 4),
                    ("judgment".into(), 4),
                    ("run".into(), 1)
                ])
            );
            let run = &rs[0];
            assert_eq!(run["value_semantics"], "score");
            assert_eq!(run["system"]["version"], "probe@0 mockllm/model");
            assert_eq!(run["grader"]["id"], "includes");
            let values: BTreeMap<(String, u64), Json> = rs
                .iter()
                .filter(|r| r["record"] == "judgment")
                .map(|r| {
                    (
                        (
                            r["item_id"].as_str().unwrap().to_owned(),
                            r["gen_replicate"].as_u64().unwrap(),
                        ),
                        r["value"].clone(),
                    )
                })
                .collect();
            assert_eq!(values[&("s1".to_owned(), 0)], json!(0));
            assert_eq!(values[&("s2".to_owned(), 1)], json!(1));
            assert!(bodies.bodies.contains(&json!("Say yes")));
            assert!(bodies.bodies.contains(&json!("Default")));
            assert!(rs.iter().all(|r| r["record"] != "label"));
        }
    }

    #[test]
    fn unfinished_and_unscored_logs_are_refused() {
        let (mut log, samples) = read_log(JSON, false, &Limits::default()).unwrap();
        let mut b = Recorder::default();
        let unscored: Vec<Json> = samples
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.as_object_mut().unwrap().remove("scores");
                s
            })
            .collect();
        assert!(matches!(
            records(&log, &unscored, &Options::default(), &mut b),
            Err(InspectError::NoScores)
        ));
        log["invalidated"] = json!(true);
        let rs = records(&log, &samples, &Options::default(), &mut b).unwrap();
        assert!(
            rs.iter()
                .filter(|r| r["record"] == "judgment")
                .all(|r| r["missing"]["reason"] == "invalidated")
        );
        log["status"] = json!("cancelled");
        assert!(matches!(
            records(&log, &samples, &Options::default(), &mut b),
            Err(InspectError::Status(_))
        ));
    }

    #[test]
    fn unscored_and_unmapped_values_are_missing() {
        let opts = Options::default();
        assert_eq!(
            value_of(&json!({"value": null, "reason": "refusal"}), &opts).unwrap_err()["reason"],
            "refused"
        );
        assert_eq!(
            value_of(&json!({"value": "N"}), &opts).unwrap_err()["reason"],
            "invalid_output"
        );
        assert_eq!(value_of(&json!({"value": true}), &opts).unwrap(), json!(1));
        let lenient = parse_lenient("x", br#"{"value": NaN, "s": "NaN", "i": -Infinity}"#).unwrap();
        assert_eq!(lenient, json!({"value": null, "s": "NaN", "i": null}));
    }

    #[test]
    fn unknown_versions_and_ambiguous_scores_are_refused() {
        let (mut log, samples) = read_log(JSON, false, &Limits::default()).unwrap();
        let mut b = Recorder::default();
        let two: Vec<Json> = samples
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s["scores"]["other"] = json!({"value": "C"});
                s
            })
            .collect();
        assert!(matches!(
            records(&log, &two, &Options::default(), &mut b),
            Err(InspectError::WhichScore(_))
        ));
        log["version"] = json!(3);
        assert!(matches!(
            records(&log, &samples, &Options::default(), &mut b),
            Err(InspectError::Version(_))
        ));
    }
}
