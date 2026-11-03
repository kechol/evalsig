//! The promptfoo adapter (SPEC §5.2 ④): `promptfoo eval -o out.json` results
//! (`results.version` 2 and 3) and the local SQLite database, opened read-only.
//!
//! - promptfoo has no stable test ID, so an item is `testCase.metadata.testCaseId`, or
//!   a hash of the test's description and vars. `input_ref` is the vars.
//! - A run is one prompt of one provider (`results.prompts[promptIdx]`): the provider
//!   is the system and the prompt's hash its version; the assertion types are the
//!   grader.
//! - `--repeat` leaves no replicate number, so the replicates of a (run, item) are
//!   numbered in `testIdx` order.
//! - Pass/fail grades the output: value semantics `score`, output type `binary`.
//!   `failureReason` 2 (an error) and failed graders are missing, never a fail.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Map, Value as Json, json};

use crate::mapping::Host;

/// The `results.version` values this adapter reads.
pub const SUPPORTED_VERSIONS: [u64; 2] = [2, 3];

#[derive(Debug, thiserror::Error)]
pub enum PromptfooError {
    #[error("not promptfoo results: {0}")]
    Json(String),
    #[error("unsupported promptfoo results version {0}; this evalsig reads versions 2 and 3")]
    Version(Json),
    #[error("promptfoo database: {0}")]
    Database(String),
    #[error("result {index}: {message}")]
    Result { index: usize, message: String },
}

/// The evaluation in `out.json` form, from the JSON output.
pub fn parse(bytes: &[u8]) -> Result<Json, PromptfooError> {
    serde_json::from_slice(bytes).map_err(|e| PromptfooError::Json(e.to_string()))
}

/// The evaluation in `out.json` form, from the local database: `eval` by ID, or the
/// most recent.
pub fn read_database(path: &Path, eval: Option<&str>) -> Result<Json, PromptfooError> {
    let db = |e: rusqlite::Error| PromptfooError::Database(e.to_string());
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db)?;
    let parse_col = |s: Option<String>| -> Json {
        s.and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Json::Null)
    };
    let (id, config, prompts): (String, Option<String>, Option<String>) = match eval {
        Some(id) => conn.query_row(
            "SELECT id, config, prompts FROM evals WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ),
        None => conn.query_row(
            "SELECT id, config, prompts FROM evals ORDER BY created_at DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ),
    }
    .map_err(db)?;
    let mut stmt = conn
        .prepare(
            "SELECT test_idx, prompt_idx, test_case, prompt, provider, response, grading_result, \
             named_scores, error, failure_reason, success, score FROM eval_results \
             WHERE eval_id = ?1 ORDER BY test_idx, prompt_idx",
        )
        .map_err(db)?;
    let rows = stmt
        .query_map([&id], |r| {
            let test_case = parse_col(r.get(2)?);
            Ok(json!({
                "testIdx": r.get::<_, i64>(0)?,
                "promptIdx": r.get::<_, i64>(1)?,
                "vars": test_case["vars"].clone(),
                "testCase": test_case,
                "prompt": parse_col(r.get(3)?),
                "provider": parse_col(r.get(4)?),
                "response": parse_col(r.get(5)?),
                "gradingResult": parse_col(r.get(6)?),
                "namedScores": parse_col(r.get(7)?),
                "error": r.get::<_, Option<String>>(8)?,
                "failureReason": r.get::<_, Option<i64>>(9)?,
                "success": r.get::<_, Option<bool>>(10)?,
                "score": r.get::<_, Option<f64>>(11)?,
            }))
        })
        .map_err(db)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db)?;
    Ok(json!({
        "evalId": id,
        "config": parse_col(config),
        "results": {"version": 3, "prompts": parse_col(prompts), "results": rows},
    }))
}

fn hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex()[..16].to_owned()
}

/// The item key: the test's own ID, or a hash of its description and vars.
fn item_id(result: &Json) -> String {
    let case = &result["testCase"];
    if let Some(id) = case["metadata"]["testCaseId"].as_str() {
        return id.to_owned();
    }
    let vars = if case["vars"].is_null() {
        &result["vars"]
    } else {
        &case["vars"]
    };
    // serde_json maps keep their keys sorted, so equal tests give equal bytes.
    let key = json!({"description": case["description"], "vars": vars});
    format!("pf-{}", hex(key.to_string().as_bytes()))
}

fn grader(results: &[&Json]) -> Json {
    let types: BTreeSet<String> = results
        .iter()
        .flat_map(|r| {
            r["gradingResult"]["componentResults"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter_map(|c| c["assertion"]["type"].as_str().map(str::to_owned))
        .collect();
    let model = types
        .iter()
        .any(|t| t.starts_with("llm-") || t.starts_with("model-") || t == "factuality");
    json!({
        "id": if types.is_empty() { "promptfoo".to_owned() } else { types.into_iter().collect::<Vec<_>>().join("+") },
        "kind": if model { "model" } else { "rule" },
    })
}

fn missing(result: &Json) -> Option<Json> {
    if result["failureReason"].as_i64() == Some(2) {
        return Some(json!({
            "reason": "error",
            "detail": result["error"].as_str().unwrap_or("provider error"),
        }));
    }
    let grader_failed = result["gradingResult"]["componentResults"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|c| c["metadata"]["graderError"] == true);
    if grader_failed {
        return Some(json!({"reason": "grader_failed", "detail": "a grader failed"}));
    }
    match &result["success"] {
        Json::Bool(_) => None,
        _ => Some(json!({"reason": "invalid_output", "detail": "no pass or fail"})),
    }
}

/// The canonical records of one evaluation, as record JSON for the importer to
/// validate.
pub fn records(eval: &Json, host: &mut dyn Host) -> Result<Vec<Json>, PromptfooError> {
    let results = &eval["results"];
    if !SUPPORTED_VERSIONS
        .iter()
        .any(|v| results["version"].as_u64() == Some(*v))
    {
        return Err(PromptfooError::Version(results["version"].clone()));
    }
    let eval_id = eval["evalId"].as_str().unwrap_or("promptfoo");
    let rows: Vec<&Json> = results["results"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let prompts = results["prompts"].as_array().cloned().unwrap_or_default();
    // Group by run, then item; within each, testIdx order gives the replicates.
    let mut runs: BTreeMap<i64, Vec<(i64, usize, &Json)>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        let prompt = r["promptIdx"].as_i64().unwrap_or(0);
        let test = r["testIdx"].as_i64().unwrap_or(i as i64);
        runs.entry(prompt).or_default().push((test, i, r));
    }
    let body = |host: &mut dyn Host, v: &Json, index: usize| {
        host.body(v)
            .map_err(|message| PromptfooError::Result { index, message })
    };
    let mut out = Vec::new();
    for (prompt_idx, mut members) in runs {
        members.sort_by_key(|(test, i, _)| (*test, *i));
        let first = members[0].2;
        let prompt = prompts
            .get(prompt_idx as usize)
            .cloned()
            .unwrap_or(Json::Null);
        let provider = prompt["provider"]
            .as_str()
            .or_else(|| {
                first["provider"]["label"]
                    .as_str()
                    .filter(|l| !l.is_empty())
            })
            .or_else(|| first["provider"]["id"].as_str())
            .unwrap_or("unknown")
            .to_owned();
        let version = prompt["id"]
            .as_str()
            .or_else(|| first["promptId"].as_str())
            .map(str::to_owned)
            .or_else(|| prompt["raw"].as_str().map(|raw| hex(raw.as_bytes())))
            .unwrap_or_else(|| "unknown".into());
        let run_id = format!("{eval_id}/{prompt_idx}");
        let results: Vec<&Json> = members.iter().map(|(_, _, r)| *r).collect();
        out.push(json!({
            "record": "run",
            "run_id": run_id,
            "system": {"id": provider, "version": version},
            "grader": grader(&results),
            "value_semantics": "score",
            "output_type": "binary",
        }));
        let mut replicate: BTreeMap<String, u32> = BTreeMap::new();
        for (_, index, r) in members {
            let id = item_id(r);
            let n = replicate.entry(id.clone()).or_insert(0);
            let gen_replicate = *n;
            *n += 1;
            let mut item = Map::new();
            item.insert("record".into(), json!("item"));
            item.insert("item_id".into(), json!(id));
            let strata: Map<String, Json> = r["testCase"]["metadata"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, v)| {
                    *k != "testCaseId"
                        && matches!(v, Json::String(_) | Json::Number(_) | Json::Bool(_))
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !strata.is_empty() {
                item.insert("strata".into(), Json::Object(strata));
            }
            let vars = if r["testCase"]["vars"].is_null() {
                &r["vars"]
            } else {
                &r["testCase"]["vars"]
            };
            if !vars.is_null() {
                item.insert("input_ref".into(), json!(body(host, vars, index)?));
            }
            out.push(Json::Object(item));
            let mut j = Map::new();
            j.insert("record".into(), json!("judgment"));
            j.insert("item_id".into(), json!(id));
            j.insert("run_id".into(), json!(run_id));
            j.insert("gen_replicate".into(), json!(gen_replicate));
            j.insert("output_type".into(), json!("binary"));
            match missing(r) {
                Some(m) => {
                    j.insert("missing".into(), m);
                }
                None => {
                    j.insert("value".into(), r["success"].clone());
                }
            }
            let output = &r["response"]["output"];
            if !output.is_null() {
                j.insert("output_ref".into(), json!(body(host, output, index)?));
            }
            out.push(Json::Object(j));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &[u8] = include_bytes!("../../../tests/fixtures/harness/promptfoo/out.json");

    struct Bodies(Vec<Json>);

    impl Host for Bodies {
        fn arg(&self, _: &str) -> Option<&str> {
            None
        }
        fn read(&mut self, _: &str) -> Result<Vec<u8>, String> {
            Err("no files".into())
        }
        fn body(&mut self, v: &Json) -> Result<String, String> {
            self.0.push(v.clone());
            Ok(format!("blake3:{:064x}", self.0.len()))
        }
    }

    fn judgments(rs: &[Json]) -> Vec<(String, u64, Json)> {
        let mut v: Vec<_> = rs
            .iter()
            .filter(|r| r["record"] == "judgment")
            .map(|r| {
                (
                    r["item_id"].as_str().unwrap().to_owned(),
                    r["gen_replicate"].as_u64().unwrap(),
                    r["value"].clone(),
                )
            })
            .collect();
        v.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        v
    }

    #[test]
    fn the_fixture_imports_with_restored_replicates() {
        let eval = parse(OUT).unwrap();
        let mut b = Bodies(Vec::new());
        let rs = records(&eval, &mut b).unwrap();
        let runs: Vec<&Json> = rs.iter().filter(|r| r["record"] == "run").collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["system"]["id"], "echo");
        assert_eq!(runs[0]["grader"], json!({"id": "contains", "kind": "rule"}));
        let js = judgments(&rs);
        assert_eq!(js.len(), 4);
        // Two items, each with replicates 0 and 1; "yes" passes, "no" fails.
        let ids: BTreeSet<&String> = js.iter().map(|j| &j.0).collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(js.iter().map(|j| j.1).collect::<Vec<_>>(), [0, 1, 0, 1]);
        let values: BTreeSet<String> = js.iter().map(|j| j.2.to_string()).collect();
        assert_eq!(
            values,
            BTreeSet::from(["true".to_owned(), "false".to_owned()])
        );
        assert!(b.0.contains(&json!({"q": "yes"})));
        assert!(rs.iter().all(|r| r["record"] != "label"));
    }

    #[test]
    fn version_2_results_read_the_same_rows() {
        let mut eval = parse(OUT).unwrap();
        eval["results"]["version"] = json!(2);
        for r in eval["results"]["results"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("testCase");
        }
        let rs = records(&eval, &mut Bodies(Vec::new())).unwrap();
        assert_eq!(judgments(&rs).len(), 4);
        eval["results"]["version"] = json!(4);
        assert!(matches!(
            records(&eval, &mut Bodies(Vec::new())),
            Err(PromptfooError::Version(_))
        ));
    }

    #[test]
    fn errors_and_failed_graders_are_missing_not_failures() {
        let error = json!({"failureReason": 2, "success": false, "error": "timeout"});
        assert_eq!(missing(&error).unwrap()["reason"], "error");
        let grader = json!({"failureReason": 1, "success": false, "gradingResult": {"componentResults": [{"pass": false, "metadata": {"graderError": true}}]}});
        assert_eq!(missing(&grader).unwrap()["reason"], "grader_failed");
        let assert_fail = json!({"failureReason": 1, "success": false, "error": "expected maybe"});
        assert!(missing(&assert_fail).is_none());
    }

    #[test]
    fn the_database_is_read_like_the_json_output() {
        let eval = parse(OUT).unwrap();
        let dir = std::env::temp_dir().join(format!("evalsig-pf-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        let conn = rusqlite::Connection::open(&dir).unwrap();
        conn.execute_batch(
            "CREATE TABLE evals (id TEXT, created_at INTEGER, config TEXT, prompts TEXT);
             CREATE TABLE eval_results (eval_id TEXT, test_idx INTEGER, prompt_idx INTEGER, test_case TEXT,
               prompt TEXT, provider TEXT, response TEXT, grading_result TEXT, named_scores TEXT, error TEXT,
               failure_reason INTEGER, success INTEGER, score REAL);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO evals VALUES (?1, 1, '{}', ?2)",
            rusqlite::params![
                eval["evalId"].as_str(),
                eval["results"]["prompts"].to_string()
            ],
        )
        .unwrap();
        for r in eval["results"]["results"].as_array().unwrap() {
            conn.execute(
                "INSERT INTO eval_results VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    eval["evalId"].as_str(),
                    r["testIdx"].as_i64(),
                    r["promptIdx"].as_i64(),
                    r["testCase"].to_string(),
                    r["prompt"].to_string(),
                    r["provider"].to_string(),
                    r["response"].to_string(),
                    r["gradingResult"].to_string(),
                    r["namedScores"].to_string(),
                    r["error"].as_str(),
                    r["failureReason"].as_i64(),
                    r["success"].as_bool(),
                    r["score"].as_f64(),
                ],
            )
            .unwrap();
        }
        drop(conn);
        let from_db = read_database(&dir, None).unwrap();
        std::fs::remove_file(&dir).unwrap();
        let a = judgments(&records(&from_db, &mut Bodies(Vec::new())).unwrap());
        let b = judgments(&records(&eval, &mut Bodies(Vec::new())).unwrap());
        assert_eq!(a, b);
    }
}
