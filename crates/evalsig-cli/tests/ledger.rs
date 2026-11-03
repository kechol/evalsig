//! `ledger verify` detects the three tampering scenarios of the audit design, and the
//! example ledgers verify.

use std::path::Path;

use assert_cmd::Command;
use evalsig_store::ledger::{Entry, entry_hash};
use serde_json::Value;

const REFEREE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/ledger.jsonl");
const AGENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/ledger-agent.jsonl"
);

fn verify(path: &Path, extra: &[&str]) -> (i32, Value) {
    let out = Command::cargo_bin("evalsig")
        .unwrap()
        .args(["--format", "json", "ledger", "verify", "--ledger"])
        .arg(path)
        .args(extra)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        serde_json::from_slice(&out.stdout).unwrap(),
    )
}

fn lines() -> Vec<Entry> {
    std::fs::read_to_string(REFEREE)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn write(dir: &Path, entries: &[Entry]) -> std::path::PathBuf {
    let p = dir.join("ledger.jsonl");
    let text: String = entries
        .iter()
        .map(|e| serde_json::to_string(e).unwrap() + "\n")
        .collect();
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn the_example_ledgers_verify() {
    let (code, v) = verify(Path::new(REFEREE), &["--side", "referee"]);
    assert_eq!((code, v["results"]["entries"].as_u64()), (0, Some(5)));
    let (code, v) = verify(Path::new(AGENT), &["--side", "agent"]);
    assert_eq!((code, v["results"]["entries"].as_u64()), (0, Some(3)));
}

#[test]
fn rewriting_a_result_breaks_its_hash() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = lines();
    let gate = e
        .iter_mut()
        .find(|x| format!("{:?}", x.kind) == "Gate")
        .unwrap();
    gate.summary
        .insert("result".into(), Value::String("pass".into()));
    let (code, v) = verify(&write(dir.path(), &e), &[]);
    assert_eq!(code, 5);
    assert!(
        v["results"]["problems"][0]
            .as_str()
            .unwrap()
            .contains("hash does not match")
    );
}

#[test]
fn rewriting_the_last_line_and_its_hash_is_caught_by_the_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = lines();
    let anchor = e.last().unwrap().hash.clone();
    let last = e.last_mut().unwrap();
    last.summary.insert("budget_left".into(), Value::from(5));
    last.hash = entry_hash(last).unwrap();
    let path = write(dir.path(), &e);
    // The chain alone looks intact...
    assert_eq!(verify(&path, &[]).0, 0);
    // ...but the anchor recorded before the rewrite is no longer in it.
    let (code, v) = verify(&path, &["--anchor", &anchor]);
    assert_eq!(code, 5);
    assert!(
        v["results"]["problems"][0]
            .as_str()
            .unwrap()
            .contains("anchor")
    );
}

#[test]
fn rewriting_an_inner_line_and_its_hash_breaks_the_next_link() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = lines();
    e[0].summary.insert("seed".into(), Value::from(1));
    e[0].hash = entry_hash(&e[0]).unwrap();
    let (code, v) = verify(&write(dir.path(), &e), &[]);
    assert_eq!(code, 5);
    assert!(
        v["results"]["problems"][0]
            .as_str()
            .unwrap()
            .contains("line 2: prev")
    );
}

#[test]
fn anchor_and_show_report_the_head() {
    let out = Command::cargo_bin("evalsig")
        .unwrap()
        .args(["--format", "json", "ledger", "anchor", "--ledger", REFEREE])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["results"]["head"], lines().last().unwrap().hash.as_str());
    let out = Command::cargo_bin("evalsig")
        .unwrap()
        .args([
            "--format", "json", "ledger", "show", "--last", "2", "--ledger", REFEREE,
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["results"]["entries"].as_array().unwrap().len(), 2);
    assert_eq!(v["results"]["count"], 5);
}

#[test]
fn entries_in_the_wrong_ledger_are_reported() {
    let (code, _) = verify(Path::new(REFEREE), &["--side", "agent"]);
    assert_eq!(code, 5);
}
