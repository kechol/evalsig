//! The referee's protected paths (SPEC §8.0 "Protects", §13.2 "Judge path tests";
//! `docs/referee.md`): one test per path an agent could use to pass the gate without
//! a real improvement or to learn the holdout's labels.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{Value, json};

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");

fn gate_toml(min_items: usize, budget: u32, accuracy_rule: &str) -> String {
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
{accuracy_rule}
sided = "one"

[holdout]
budget = {budget}
"#
    )
}

const NOT_WORSE: &str = "rule = \"not_worse_than\"\nvalue = -0.3";

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

struct World {
    referee: tempfile::TempDir,
    agent: tempfile::TempDir,
    home: tempfile::TempDir,
    holdout: String,
    approved: String,
    held: BTreeSet<String>,
}

fn world(gate: &str, mapping: Option<&Path>) -> World {
    world_from(&lines(), gate, mapping)
}

/// A referee project from `referee_records`, a sealed holdout, an agent that holds
/// only the remaining items' labels, and approved conditions.
fn world_from(referee_records: &[Value], gate: &str, mapping: Option<&Path>) -> World {
    let referee = project(&jsonl(referee_records));
    let home = tempfile::tempdir().unwrap();
    evalsig(referee.path())
        .args(["referee", "init"])
        .arg(home.path())
        .assert()
        .success();
    let remaining = referee.path().join("remaining.json");
    let out = evalsig(referee.path())
        .args(["--format", "json", "split", "--holdout", "--referee"])
        .arg(home.path())
        .arg("--out")
        .arg(&remaining)
        .output()
        .unwrap();
    let holdout = json(&out)["results"]["holdout"]["hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&remaining).unwrap()).unwrap();
    let kept: BTreeSet<&str> = kept["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap())
        .collect();
    let held: BTreeSet<String> = lines()
        .iter()
        .filter(|v| v["record"] == "item" && !kept.contains(v["item_id"].as_str().unwrap()))
        .map(|v| v["item_id"].as_str().unwrap().to_owned())
        .collect();
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
    let mut w = World {
        referee,
        agent,
        home,
        holdout,
        approved: String::new(),
        held,
    };
    w.approved = approve(&w, gate, mapping);
    w
}

/// Draft on the agent's side, approve on the referee's; returns the approved hash.
fn approve(w: &World, gate: &str, mapping: Option<&Path>) -> String {
    let g = w.agent.path().join("gate.toml");
    std::fs::write(&g, gate).unwrap();
    let d = json(
        &evalsig(w.agent.path())
            .args(["--format", "json", "preregister", "--draft"])
            .arg(&g)
            .output()
            .unwrap(),
    );
    let mut c = evalsig(w.referee.path());
    c.args([
        "--format",
        "json",
        "preregister",
        "approve",
        d["results"]["draft"].as_str().unwrap(),
        "--holdout",
        &w.holdout,
        "--from",
    ])
    .arg(w.agent.path())
    .arg("--referee")
    .arg(w.home.path());
    if let Some(m) = mapping {
        c.arg("--mapping").arg(m);
    }
    let a = c.output().unwrap();
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    json(&a)["results"]["approved"].as_str().unwrap().to_owned()
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

/// A distinct but equivalent candidate: only the run's attributes differ.
fn variant(dir: &Path, k: u32) -> PathBuf {
    candidate(dir, &format!("variant-{k}.jsonl"), |rows| {
        rows[0]["attrs"] = json!({ "try": k })
    })
}

fn submit_to(w: &World, approved: &str, records: &Path, extra: &[&str]) -> std::process::Output {
    evalsig(w.referee.path())
        .args([
            "--format",
            "json",
            "gate",
            "--approved",
            approved,
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

fn submit(w: &World, records: &Path) -> std::process::Output {
    submit_to(w, &w.approved, records, &[])
}

fn code(out: &std::process::Output) -> Option<i32> {
    out.status.code()
}

fn referee_ledger(w: &World) -> Vec<Value> {
    std::fs::read_to_string(w.home.path().join("ledger.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn accesses(w: &World) -> usize {
    referee_ledger(w)
        .iter()
        .filter(|e| e["kind"] == "holdout_access")
        .count()
}

/// Every reference label of each item.
fn references() -> BTreeMap<String, Vec<Value>> {
    let mut m: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for v in lines().into_iter().filter(|v| v["record"] == "label") {
        m.entry(v["item_id"].as_str().unwrap().to_owned())
            .or_default()
            .push(v["value"].clone());
    }
    m
}

#[test]
fn referee_item_omission() {
    let w = world(&gate_toml(10, 3, NOT_WORSE), None);
    // Leaving out the items the candidate gets wrong is refused before labels are read.
    let refs = references();
    let records = candidate(w.referee.path(), "omit.jsonl", |rows| {
        rows.retain(|r| {
            let id = r["item_id"].as_str().unwrap_or_default();
            r["record"] == "run"
                || !w.held.contains(id)
                || refs[id].iter().all(|l| *l == r["value"])
        })
    });
    let out = submit(&w, &records);
    assert_eq!(code(&out), Some(3));
    assert_eq!(json(&out)["error"]["code"], "incomplete_coverage");
    // Judgments for another task do not cover the holdout either.
    let other = candidate(w.referee.path(), "task.jsonl", |rows| {
        for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
            r["task"] = "other".into();
        }
    });
    assert_eq!(code(&submit(&w, &other)), Some(3));
    assert_eq!(accesses(&w), 1, "only the task mix-up passed coverage");
}

#[test]
fn referee_missing_reason_manipulation() {
    // A high bar the candidate meets only if its wrong judgments drop out.
    let rule = "rule = \"lower_bound_at_least\"\nvalue = 0.85";
    let w = world(&gate_toml(10, 10, rule), None);
    let refs = references();
    for (k, reason) in [
        "excluded",
        "not_run",
        "invalidated",
        "grader_failed",
        "error",
    ]
    .into_iter()
    .enumerate()
    {
        let records = candidate(w.referee.path(), &format!("m{k}.jsonl"), |rows| {
            for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
                let id = r["item_id"].as_str().unwrap();
                if w.held.contains(id) && refs[id].iter().any(|l| *l != r["value"]) {
                    let o = r.as_object_mut().unwrap();
                    o.remove("value");
                    o.insert(
                        "missing".into(),
                        json!({ "reason": reason, "detail": "manipulated" }),
                    );
                }
            }
        });
        // Enough judgments are left that dropping the missing ones would clear the bar.
        let left = std::fs::read_to_string(&records)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|r| {
                r["record"] == "judgment"
                    && r.get("value").is_some()
                    && w.held.contains(r["item_id"].as_str().unwrap())
            })
            .count();
        assert!(left >= 40, "{left}");
        let out = submit(&w, &records);
        assert_eq!(
            code(&out),
            Some(1),
            "{reason}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(json(&out)["verdict"]["code"], "fail");
    }
}

#[test]
fn referee_attribute_rewrite() {
    let w = world(&gate_toml(10, 3, NOT_WORSE), None);
    // Item records in the candidate's file are not read: the referee's own items count.
    let records = candidate(w.referee.path(), "items.jsonl", |rows| {
        for id in &w.held {
            rows.push(json!({ "record": "item", "item_id": id, "weight": 0.001 }));
        }
    });
    assert_eq!(code(&submit(&w, &records)), Some(0));
    // A holdout item's attributes changed in the referee's store: refused.
    let id = w.held.iter().next().unwrap();
    let f = w.referee.path().join("rewrite.jsonl");
    std::fs::write(
        &f,
        format!(
            "{}\n",
            json!({ "record": "item", "item_id": id, "cluster": "thread-000", "weight": 50.0 })
        ),
    )
    .unwrap();
    evalsig(w.referee.path())
        .arg("import")
        .arg(&f)
        .assert()
        .success();
    let out = submit(&w, &variant(w.referee.path(), 1));
    assert_eq!(
        code(&out),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json(&out)["error"]["code"], "does_not_match_approval");
}

#[test]
fn referee_config_rewrite() {
    let w = world(&gate_toml(10, 5, NOT_WORSE), None);
    let before = json(&submit(&w, &variant(w.referee.path(), 1)));
    // Positive definition, missing handling, level and seed all rewritten after approval.
    std::fs::write(
        w.referee.path().join(".evalsig/config.toml"),
        r#"[stats]
seed = 1
level = 0.5

[missing]
system = "exclude"
measurement = "exclude"

[metrics]
positive = { equals = false }
"#,
    )
    .unwrap();
    let after = json(&submit(&w, &variant(w.referee.path(), 2)));
    assert_eq!(before["verdict"], after["verdict"]);
    assert_eq!(
        before["results"]["conditions"],
        after["results"]["conditions"]
    );
}

#[test]
fn gate_reads_no_project_config() {
    let w = world(&gate_toml(10, 3, NOT_WORSE), None);
    std::fs::write(
        w.referee.path().join(".evalsig/config.toml"),
        "this is [not toml",
    )
    .unwrap();
    let out = submit(&w, &variant(w.referee.path(), 1));
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn referee_value_map_rewrite() {
    let approved_map = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        approved_map.path(),
        "schema = \"mapping/1\"\n# value_map: yes -> true\n",
    )
    .unwrap();
    let w = world(&gate_toml(10, 3, NOT_WORSE), Some(approved_map.path()));
    let hash =
        |p: &Path| evalsig_store::hash::ContentHash::blake3(&std::fs::read(p).unwrap()).to_string();
    let rewritten = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        rewritten.path(),
        "schema = \"mapping/1\"\n# value_map: yes -> false\n",
    )
    .unwrap();
    let records = variant(w.referee.path(), 1);
    let out = submit_to(
        &w,
        &w.approved,
        &records,
        &["--mapping-hash", &hash(rewritten.path())],
    );
    assert_eq!(code(&out), Some(3));
    assert_eq!(json(&out)["error"]["code"], "does_not_match_approval");
    let out = submit_to(
        &w,
        &w.approved,
        &variant(w.referee.path(), 2),
        &["--mapping-hash", &hash(approved_map.path())],
    );
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn referee_grader_version_mismatch() {
    let w = world(&gate_toml(10, 3, NOT_WORSE), None);
    for (k, grader) in [
        json!({ "id": "rule-check", "version": "2", "kind": "rule" }),
        json!({ "id": "other-check", "version": "1", "kind": "rule" }),
    ]
    .into_iter()
    .enumerate()
    {
        let records = candidate(w.referee.path(), &format!("g{k}.jsonl"), |rows| {
            rows[0]["grader"] = grader.clone()
        });
        let out = submit(&w, &records);
        assert_eq!(code(&out), Some(3));
        assert_eq!(json(&out)["error"]["code"], "does_not_match_approval");
    }
}

#[test]
fn referee_reapproval_keeps_budget() {
    let w = world(&gate_toml(10, 2, NOT_WORSE), None);
    assert_eq!(code(&submit(&w, &variant(w.referee.path(), 1))), Some(0));
    // Rewritten conditions, approved again on the same sealed holdout.
    let again = approve(
        &w,
        &gate_toml(10, 2, "rule = \"not_worse_than\"\nvalue = -0.5"),
        None,
    );
    assert_ne!(again, w.approved);
    let out = submit_to(&w, &again, &variant(w.referee.path(), 2), &[]);
    assert_eq!(code(&out), Some(0));
    assert_eq!(json(&out)["results"]["budget"]["used"], 2);
    let out = submit_to(&w, &again, &variant(w.referee.path(), 3), &[]);
    assert_eq!(code(&out), Some(2));
    assert_eq!(json(&out)["verdict"]["code"], "budget_exhausted");
}

#[test]
fn referee_same_candidate_resubmission() {
    let w = world(&gate_toml(10, 3, NOT_WORSE), None);
    let records = variant(w.referee.path(), 1);
    let first = json(&submit(&w, &records));
    // The same records in another order are the same snapshot.
    let text = std::fs::read_to_string(&records).unwrap();
    let mut rows: Vec<&str> = text.lines().collect();
    rows.reverse();
    let reordered = w.referee.path().join("reordered.jsonl");
    std::fs::write(&reordered, rows.join("\n")).unwrap();
    for r in [&records, &reordered] {
        let out = json(&submit(&w, r));
        assert_eq!(out["warnings"][0]["code"], "resubmission");
        assert_eq!(out["verdict"], first["verdict"]);
        assert_eq!(out["results"]["budget"], first["results"]["budget"]);
    }
    assert_eq!(accesses(&w), 1);
}

#[test]
fn referee_min_items_boundary() {
    // The referee's baseline lacks five holdout items, so the rest are paired.
    let held = world(&gate_toml(10, 1, NOT_WORSE), None).held;
    let dropped: BTreeSet<&String> = held.iter().take(5).collect();
    let rows: Vec<Value> = lines()
        .into_iter()
        .filter(|v| {
            !(v["record"] == "judgment"
                && v["run_id"] == "triage-v1"
                && dropped.contains(&v["item_id"].as_str().unwrap().to_owned()))
        })
        .collect();
    let paired = held.len() - dropped.len();
    let at = world_from(&rows, &gate_toml(paired, 1, NOT_WORSE), None);
    let out = submit(&at, &variant(at.referee.path(), 1));
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let above = world_from(&rows, &gate_toml(paired + 1, 2, NOT_WORSE), None);
    let out = submit(&above, &variant(above.referee.path(), 1));
    assert_eq!(code(&out), Some(4));
    assert_eq!(json(&out)["verdict"]["code"], "insufficient_data");
    // A candidate that gets every holdout item wrong is short in the same way: the
    // answer says nothing about the labels.
    let refs = references();
    let wrong = candidate(above.referee.path(), "wrong.jsonl", |rows| {
        for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
            let id = r["item_id"].as_str().unwrap();
            r["value"] = (refs[id][0] != json!(true)).into();
        }
    });
    let out = submit(&above, &wrong);
    assert_eq!(code(&out), Some(4));
}

#[test]
fn referee_warnings_withheld() {
    let w = world(&gate_toml(10, 3, NOT_WORSE), None);
    let records = candidate(w.referee.path(), "warn.jsonl", |rows| {
        let j = rows.iter_mut().find(|r| r["record"] == "judgment").unwrap();
        let o = j.as_object_mut().unwrap();
        o.remove("value");
        o.insert("missing".into(), json!({ "reason": "other" }));
    });
    let out = json(&submit(&w, &records));
    assert!(
        out["warnings"].as_array().unwrap().is_empty(),
        "{}",
        out["warnings"]
    );
    let gate = referee_ledger(&w)
        .into_iter()
        .rfind(|e| e["kind"] == "gate")
        .unwrap();
    assert_eq!(
        gate["summary"]["withheld_warnings"],
        json!(["unexplained_missing"])
    );
}

#[test]
fn referee_forged_pass_is_not_in_referee_ledger() {
    let w = world(&gate_toml(10, 3, NOT_WORSE), None);
    let refs = references();
    let wrong = candidate(w.referee.path(), "wrong.jsonl", |rows| {
        for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
            let id = r["item_id"].as_str().unwrap();
            r["value"] = (refs[id][0] != json!(true)).into();
        }
    });
    let out = submit(&w, &wrong);
    assert_eq!(code(&out), Some(1));
    // The agent relays a pass: in its own ledger and in a result file it writes.
    let forged = json!({ "kind": "gate", "summary": { "verdict": "pass" } });
    let ledger = w.agent.path().join(".evalsig/ledger.jsonl");
    let mut text = std::fs::read_to_string(&ledger).unwrap();
    text.push_str(&format!("{forged}\n"));
    std::fs::write(&ledger, text).unwrap();
    std::fs::write(w.agent.path().join("gate-result.json"), forged.to_string()).unwrap();
    // People read the referee's ledger and results, where only the real verdict is.
    let verdicts: Vec<Value> = referee_ledger(&w)
        .into_iter()
        .filter(|e| e["kind"] == "gate")
        .map(|e| e["summary"]["verdict"].clone())
        .collect();
    assert_eq!(verdicts, vec![json!("fail")]);
    let results: Vec<_> = std::fs::read_dir(w.home.path().join("results"))
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert_eq!(results.len(), 1);
    assert!(results[0].contains("\"fail\""));
}
