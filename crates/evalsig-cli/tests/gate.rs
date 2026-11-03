//! `evalsig gate` (SPEC §8.3, §8.4; metrics spec §11.2): each submission check, the
//! budget, and resubmission of the same candidate.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");

fn gate_toml(min_items: usize, budget: u32) -> String {
    format!(
        r#"name = "triage-v2 is no worse"
hypothesis = "The new prompt keeps accuracy"
split = "holdout"
min_items = {min_items}

[compare]
baseline = "triage-v1"
candidate = "*"

[grader]
id = "rule-check"
version = "1"

[[conditions]]
metric = "accuracy"
rule = "not_worse_than"
value = -0.3
sided = "one"

[[conditions]]
metric = "missing_rate"
rule = "upper_bound_at_most"
value = 0.5
sided = "one"

[holdout]
budget = {budget}
"#
    )
}

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("EVALSIG_REFEREE")
        .arg("--project")
        .arg(dir);
    c
}

fn json(out: &std::process::Output) -> Value {
    serde_json::from_slice(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    })
    .unwrap()
}

fn project(records: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    let f = dir.path().join("records.jsonl");
    std::fs::write(&f, records).unwrap();
    evalsig(dir.path()).arg("import").arg(&f).assert().success();
    dir
}

fn lines() -> Vec<Value> {
    std::fs::read_to_string(SYNTHETIC)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn jsonl(rows: &[Value]) -> String {
    rows.iter().map(|v| format!("{v}\n")).collect()
}

/// The holdout the synthetic items give with the default seed: split depends only on
/// the items and the seed, so any project with these items cuts the same one.
fn holdout_items() -> BTreeSet<String> {
    let p = project(&std::fs::read_to_string(SYNTHETIC).unwrap());
    let home = tempfile::tempdir().unwrap();
    evalsig(p.path())
        .args(["referee", "init"])
        .arg(home.path())
        .assert()
        .success();
    let out = p.path().join("remaining.json");
    evalsig(p.path())
        .args(["split", "--holdout", "--referee"])
        .arg(home.path())
        .arg("--out")
        .arg(&out)
        .assert()
        .success();
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let kept: BTreeSet<&str> = kept["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap())
        .collect();
    lines()
        .iter()
        .filter(|v| v["record"] == "item" && !kept.contains(v["item_id"].as_str().unwrap()))
        .map(|v| v["item_id"].as_str().unwrap().to_owned())
        .collect()
}

struct World {
    referee: tempfile::TempDir,
    home: tempfile::TempDir,
    approved: String,
    held: BTreeSet<String>,
}

/// A referee project from `referee_records`, a sealed holdout, an agent that never had
/// its labels, and approved conditions.
fn world(referee_records: &[Value], gate: &str, mapping: Option<&Path>) -> World {
    let referee = project(&jsonl(referee_records));
    let home = tempfile::tempdir().unwrap();
    evalsig(referee.path())
        .args(["referee", "init"])
        .arg(home.path())
        .assert()
        .success();
    let out = evalsig(referee.path())
        .args(["--format", "json", "split", "--holdout", "--referee"])
        .arg(home.path())
        .arg("--out")
        .arg(referee.path().join("remaining.json"))
        .output()
        .unwrap();
    let holdout = json(&out)["results"]["holdout"]["hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let held = holdout_items();
    let agent_rows: Vec<Value> = lines()
        .into_iter()
        .filter(|v| v["record"] != "label" || !held.contains(v["item_id"].as_str().unwrap()))
        .collect();
    let agent = project(&jsonl(&agent_rows));
    evalsig(referee.path())
        .args(["seal", "--holdout", &holdout, "--referee"])
        .arg(home.path())
        .arg("--agent-project")
        .arg(agent.path())
        .assert()
        .success();
    let g = agent.path().join("gate.toml");
    std::fs::write(&g, gate).unwrap();
    let d = json(
        &evalsig(agent.path())
            .args(["--format", "json", "preregister", "--draft"])
            .arg(&g)
            .output()
            .unwrap(),
    );
    let mut c = evalsig(referee.path());
    c.args([
        "--format",
        "json",
        "preregister",
        "approve",
        d["results"]["draft"].as_str().unwrap(),
        "--holdout",
        &holdout,
        "--from",
    ])
    .arg(agent.path())
    .arg("--referee")
    .arg(home.path());
    if let Some(m) = mapping {
        c.arg("--mapping").arg(m);
    }
    let a = c.output().unwrap();
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    World {
        referee,
        home,
        approved: json(&a)["results"]["approved"].as_str().unwrap().to_owned(),
        held,
    }
}

/// The candidate's records: triage-v2's run and judgments, changed by `edit`.
fn candidate(dir: &Path, name: &str, edit: impl Fn(&mut Vec<Value>)) -> PathBuf {
    let mut rows: Vec<Value> = lines()
        .into_iter()
        .filter(|v| {
            (v["record"] == "run" || v["record"] == "judgment") && v["run_id"] == "triage-v2"
        })
        .collect();
    edit(&mut rows);
    let p = dir.join(name);
    std::fs::write(&p, jsonl(&rows)).unwrap();
    p
}

fn submit(w: &World, records: &Path, extra: &[&str]) -> std::process::Output {
    evalsig(w.referee.path())
        .args([
            "--format",
            "json",
            "gate",
            "--approved",
            &w.approved,
            "--candidate",
            "triage-v2",
            "--records",
        ])
        .arg(records)
        .arg("--referee")
        .arg(w.home.path())
        .args(extra)
        .output()
        .unwrap()
}

fn accesses(w: &World) -> usize {
    std::fs::read_to_string(w.home.path().join("ledger.jsonl"))
        .unwrap()
        .lines()
        .filter(|l| l.contains("\"holdout_access\""))
        .count()
}

#[test]
fn a_passing_candidate_learns_only_verdicts_and_a_resubmission_costs_nothing() {
    let w = world(&lines(), &gate_toml(10, 3), None);
    let records = candidate(w.referee.path(), "cand.jsonl", |_| {});
    let out = submit(&w, &records, &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v = json(&out);
    assert_eq!(v["verdict"]["code"], "pass");
    assert_eq!(
        v["results"]["budget"],
        serde_json::json!({"total": 3, "used": 1, "remaining": 2})
    );
    // Verdicts per condition, nothing else: no interval, no value, no warning.
    for c in v["results"]["conditions"].as_array().unwrap() {
        let keys: BTreeSet<&str> = c.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, BTreeSet::from(["metric", "rule", "value", "passed"]));
    }
    assert!(
        v["warnings"].as_array().unwrap().is_empty(),
        "{}",
        v["warnings"]
    );
    let ledger = std::fs::read_to_string(w.home.path().join("ledger.jsonl")).unwrap();
    let gate: Value = serde_json::from_str(
        ledger
            .lines()
            .rfind(|l| l.contains("\"kind\":\"gate\""))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(gate["summary"]["seal_void"], false);
    assert!(gate["summary"]["withheld_warnings"].is_array());

    let again = submit(&w, &records, &[]);
    assert_eq!(again.status.code(), Some(0));
    let v = json(&again);
    assert_eq!(v["warnings"][0]["code"], "resubmission");
    assert_eq!(accesses(&w), 1);
}

#[test]
fn missing_holdout_items_are_refused_before_any_budget_is_spent() {
    let w = world(&lines(), &gate_toml(10, 3), None);
    let left_out = w.held.iter().next().unwrap().clone();
    let records = candidate(w.referee.path(), "short.jsonl", |rows| {
        rows.retain(|r| r["item_id"] != left_out.as_str())
    });
    let out = submit(&w, &records, &[]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(json(&out)["error"]["code"], "incomplete_coverage");
    assert_eq!(accesses(&w), 0);
}

#[test]
fn a_different_grader_or_mapping_is_refused_after_spending_budget() {
    let mapping = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(mapping.path(), "schema = \"mapping/1\"\n").unwrap();
    let w = world(&lines(), &gate_toml(10, 5), Some(mapping.path()));
    let other = candidate(w.referee.path(), "grader.jsonl", |rows| {
        rows[0]["grader"]["version"] = "2".into()
    });
    let out = submit(&w, &other, &[]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(json(&out)["error"]["code"], "does_not_match_approval");
    assert_eq!(accesses(&w), 1);

    let records = candidate(w.referee.path(), "cand.jsonl", |_| {});
    let out = submit(&w, &records, &[]);
    assert_eq!(out.status.code(), Some(3), "no mapping hash given");
    let hash = evalsig_store::hash::ContentHash::blake3(&std::fs::read(mapping.path()).unwrap())
        .to_string();
    let out = submit(
        &w,
        &candidate(w.referee.path(), "cand2.jsonl", |rows| {
            rows[0]["attrs"] = serde_json::json!({"n": 2})
        }),
        &["--mapping-hash", &hash],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn identical_replicates_stop_the_gate() {
    let w = world(&lines(), &gate_toml(10, 3), None);
    let records = candidate(w.referee.path(), "copies.jsonl", |rows| {
        let mut extra = Vec::new();
        for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
            r["output_ref"] = format!("blake3:{}", "a".repeat(64)).into();
            let mut copy = r.clone();
            copy["gen_replicate"] = 1.into();
            extra.push(copy);
        }
        rows.extend(extra);
    });
    let out = submit(&w, &records, &[]);
    assert_eq!(
        out.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json(&out)["verdict"]["code"], "blocked");
    assert_eq!(accesses(&w), 1);
}

#[test]
fn too_few_paired_items_are_insufficient_whatever_the_labels() {
    // The referee's baseline lacks five holdout items, so only the rest are paired.
    let held = holdout_items();
    let dropped: BTreeSet<String> = held.iter().take(5).cloned().collect();
    let rows: Vec<Value> = lines()
        .into_iter()
        .filter(|v| {
            !(v["record"] == "judgment"
                && v["run_id"] == "triage-v1"
                && dropped.contains(v["item_id"].as_str().unwrap()))
        })
        .collect();
    let w = world(&rows, &gate_toml(held.len(), 3), None);
    let out = submit(&w, &candidate(w.referee.path(), "cand.jsonl", |_| {}), &[]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json(&out)["verdict"]["code"], "insufficient_data");
}

#[test]
fn the_budget_runs_out() {
    let w = world(&lines(), &gate_toml(10, 2), None);
    for k in 0..2 {
        let records = candidate(w.referee.path(), &format!("c{k}.jsonl"), |rows| {
            rows[0]["attrs"] = serde_json::json!({"try": k})
        });
        assert_eq!(submit(&w, &records, &[]).status.code(), Some(0));
    }
    let records = candidate(w.referee.path(), "c9.jsonl", |rows| {
        rows[0]["attrs"] = serde_json::json!({"try": 9})
    });
    let out = submit(&w, &records, &[]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(json(&out)["verdict"]["code"], "budget_exhausted");
    assert_eq!(accesses(&w), 2);
}

#[test]
fn model_derived_holdout_references_stop_the_gate() {
    let rows: Vec<Value> = lines()
        .into_iter()
        .map(|mut v| {
            if v["record"] == "label" {
                v["source"] = "model".into();
            }
            v
        })
        .collect();
    let w = world(&rows, &gate_toml(10, 3), None);
    let out = submit(&w, &candidate(w.referee.path(), "cand.jsonl", |_| {}), &[]);
    assert_eq!(
        out.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
