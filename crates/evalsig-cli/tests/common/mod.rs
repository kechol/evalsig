//! Shared setup for the referee's integration tests: a referee project with the
//! synthetic records, a sealed holdout, an agent without its labels, approved
//! conditions, and candidate submissions.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{Value, json};

pub const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
pub const CONFIG: &str = include_str!("../../../../examples/synthetic/config.toml");

pub fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("EVALSIG_REFEREE")
        .arg("--project")
        .arg(dir);
    c
}

pub fn json(out: &std::process::Output) -> Value {
    serde_json::from_slice(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    })
    .unwrap()
}

pub fn project(records: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    let f = dir.path().join("records.jsonl");
    std::fs::write(&f, records).unwrap();
    evalsig(dir.path()).arg("import").arg(&f).assert().success();
    dir
}

pub fn lines() -> Vec<Value> {
    std::fs::read_to_string(SYNTHETIC)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub fn jsonl(rows: &[Value]) -> String {
    rows.iter().map(|v| format!("{v}\n")).collect()
}

pub struct World {
    pub referee: tempfile::TempDir,
    pub agent: tempfile::TempDir,
    pub home: tempfile::TempDir,
    pub holdout: String,
    pub approved: String,
    pub held: BTreeSet<String>,
}

pub fn world(gate: &str, mapping: Option<&Path>) -> World {
    world_from(&lines(), gate, mapping)
}

/// A referee project from `referee_records`, a sealed holdout, an agent that holds
/// only the remaining items' labels, and approved conditions.
pub fn world_from(referee_records: &[Value], gate: &str, mapping: Option<&Path>) -> World {
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
pub fn approve(w: &World, gate: &str, mapping: Option<&Path>) -> String {
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
pub fn candidate(dir: &Path, name: &str, edit: impl Fn(&mut Vec<Value>)) -> PathBuf {
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
pub fn variant(dir: &Path, k: u32) -> PathBuf {
    candidate(dir, &format!("variant-{k}.jsonl"), |rows| {
        rows[0]["attrs"] = json!({ "try": k })
    })
}

pub fn submit_to(
    w: &World,
    approved: &str,
    records: &Path,
    extra: &[&str],
) -> std::process::Output {
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

pub fn submit(w: &World, records: &Path) -> std::process::Output {
    submit_to(w, &w.approved, records, &[])
}

pub fn code(out: &std::process::Output) -> Option<i32> {
    out.status.code()
}

pub fn referee_ledger(w: &World) -> Vec<Value> {
    std::fs::read_to_string(w.home.path().join("ledger.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub fn accesses(w: &World) -> usize {
    referee_ledger(w)
        .iter()
        .filter(|e| e["kind"] == "holdout_access")
        .count()
}
