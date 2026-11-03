//! `evalsig exclude` (SPEC §10.1, D-027): reason codes, replacement snapshots that
//! keep earlier reports verifiable, and holdout items left to the referee.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("EVALSIG_REFEREE")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

fn json_of(c: &mut Command, code: i32) -> Value {
    let out = c.assert().code(code).get_output().clone();
    serde_json::from_slice(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    })
    .unwrap()
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    evalsig(dir.path())
        .args(["import", SYNTHETIC])
        .assert()
        .success();
    dir
}

fn item_ids(dir: &Path) -> BTreeSet<String> {
    json_of(evalsig(dir).arg("items"), 0)["results"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["item_id"].as_str().unwrap().to_owned())
        .collect()
}

fn saved_report(dir: &Path, v: &Value) -> PathBuf {
    let hex = v["provenance"]["output"]
        .as_str()
        .unwrap()
        .strip_prefix("blake3:")
        .unwrap()
        .to_owned();
    dir.join(format!(".evalsig/reports/{hex}.json"))
}

#[test]
fn excluded_items_leave_every_analysis_and_earlier_reports_still_verify() {
    let dir = project();
    let before = json_of(evalsig(dir.path()).args(["report", "triage-v1"]), 0);
    let v = json_of(
        evalsig(dir.path()).args(["exclude", "t-0000", "t-0001", "--reason", "duplicate"]),
        0,
    );
    assert_eq!(v["results"]["count"], 2);
    assert_eq!(v["results"]["reason"], "duplicate");
    assert_eq!(v["warnings"][0]["code"], "reimport_restores");
    let items = item_ids(dir.path());
    assert!(!items.contains("t-0000") && !items.contains("t-0001"));
    assert_eq!(items.len(), 298);
    let after = json_of(evalsig(dir.path()).args(["report", "triage-v1"]), 0);
    assert_ne!(
        after["provenance"]["snapshots"],
        before["provenance"]["snapshots"]
    );
    // The report made before the exclusion names the superseded snapshot and verifies.
    json_of(
        evalsig(dir.path())
            .arg("verify")
            .arg(saved_report(dir.path(), &before)),
        0,
    );
    let ledger = std::fs::read_to_string(dir.path().join(".evalsig/ledger.jsonl")).unwrap();
    let line: Value = serde_json::from_str(
        ledger
            .lines()
            .rfind(|l| l.contains("\"kind\":\"exclude\""))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(line["summary"]["count"], 2);
    assert_eq!(line["summary"]["reason"], "duplicate");
    json_of(evalsig(dir.path()).args(["ledger", "verify"]), 0);
    // Importing the same file again keeps the items out.
    evalsig(dir.path())
        .args(["import", SYNTHETIC])
        .assert()
        .success();
    assert_eq!(item_ids(dir.path()).len(), 298);
}

#[test]
fn reasons_are_codes_and_items_must_exist() {
    let dir = project();
    evalsig(dir.path())
        .args(["exclude", "t-0000", "--reason", "because"])
        .assert()
        .code(64);
    let e = json_of(
        evalsig(dir.path()).args(["exclude", "t-0000", "--reason", "other"]),
        64,
    );
    assert!(e["error"]["message"].as_str().unwrap().contains("--detail"));
    json_of(
        evalsig(dir.path()).args(["exclude", "nope", "--reason", "privacy"]),
        64,
    );
    json_of(
        evalsig(dir.path()).args([
            "exclude",
            "t-0002",
            "--reason",
            "other",
            "--detail",
            "the thread was merged",
        ]),
        0,
    );
}

#[test]
fn holdout_items_are_excluded_by_the_referee_only() {
    let referee = project();
    let home = tempfile::tempdir().unwrap();
    evalsig(referee.path())
        .args(["referee", "init"])
        .arg(home.path())
        .assert()
        .success();
    let remaining = referee.path().join("remaining.json");
    let split = json_of(
        evalsig(referee.path())
            .args(["split", "--holdout", "--referee"])
            .arg(home.path())
            .arg("--out")
            .arg(&remaining),
        0,
    );
    let holdout = split["results"]["holdout"]["hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&remaining).unwrap()).unwrap();
    let kept: BTreeSet<String> = kept["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap().to_owned())
        .collect();
    let all = item_ids(referee.path());
    let held: Vec<&String> = all.iter().filter(|i| !kept.contains(*i)).collect();

    // The agent, without the holdout's labels, splits within the remaining items and
    // then tries to exclude a holdout item.
    let agent = tempfile::tempdir().unwrap();
    evalsig(agent.path()).arg("init").assert().success();
    std::fs::write(agent.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    let rows: String = std::fs::read_to_string(SYNTHETIC)
        .unwrap()
        .lines()
        .filter(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["record"] != "label" || kept.contains(v["item_id"].as_str().unwrap())
        })
        .map(|l| format!("{l}\n"))
        .collect();
    let f = agent.path().join("records.jsonl");
    std::fs::write(&f, rows).unwrap();
    evalsig(agent.path())
        .arg("import")
        .arg(&f)
        .assert()
        .success();
    evalsig(agent.path())
        .args(["split", "--within"])
        .arg(&remaining)
        .assert()
        .success();
    let e = json_of(
        evalsig(agent.path()).args(["exclude", held[0], "--reason", "duplicate"]),
        3,
    );
    assert_eq!(e["error"]["code"], "holdout_item");
    assert_eq!(item_ids(agent.path()).len(), 300);
    let mine = kept.iter().next().unwrap();
    json_of(
        evalsig(agent.path()).args(["exclude", mine, "--reason", "duplicate"]),
        0,
    );

    // The referee excludes a holdout item; one outside every holdout is the agent's.
    json_of(
        evalsig(referee.path())
            .args(["exclude", mine, "--reason", "duplicate", "--referee"])
            .arg(home.path()),
        64,
    );
    let v = json_of(
        evalsig(referee.path())
            .args(["exclude", held[0], "--reason", "invalid_input", "--referee"])
            .arg(home.path()),
        0,
    );
    assert_eq!(v["provenance"]["ledger"], "referee");
    let ledger = std::fs::read_to_string(home.path().join("ledger.jsonl")).unwrap();
    assert!(ledger.contains("\"kind\":\"exclude\""));
    // The holdout now leaves that item out, so the seal covers one item fewer.
    let sealed = json_of(
        evalsig(referee.path())
            .args(["seal", "--holdout", &holdout, "--referee"])
            .arg(home.path())
            .arg("--agent-project")
            .arg(agent.path()),
        0,
    );
    assert_eq!(sealed["results"]["items"], held.len() - 1);
}
