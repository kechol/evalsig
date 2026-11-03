//! The referee's `split --holdout` (SPEC §8.1, D-014, D-022).

use std::path::Path;

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
        .arg("--project")
        .arg(dir);
    c
}

fn json(out: &[u8]) -> Value {
    serde_json::from_slice(out).unwrap()
}

/// The referee's own project with the labelled data, and a referee home beside it.
fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
    let project = tempfile::tempdir().unwrap();
    evalsig(project.path()).arg("init").assert().success();
    std::fs::write(project.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    evalsig(project.path())
        .args(["import", SYNTHETIC])
        .assert()
        .success();
    let home = tempfile::tempdir().unwrap();
    evalsig(project.path())
        .args(["referee", "init"])
        .arg(home.path())
        .assert()
        .success();
    (project, home)
}

#[test]
fn the_agent_cannot_cut_the_holdout() {
    let (project, _home) = setup();
    let out = evalsig(project.path())
        .args(["--format", "json", "split", "--holdout", "--out"])
        .arg(project.path().join("remaining.json"))
        .assert()
        .failure()
        .code(3);
    assert_eq!(
        json(&out.get_output().stderr)["error"]["code"],
        "referee_only"
    );
    // An identity that cannot be read is no authority either.
    let empty = tempfile::tempdir().unwrap();
    evalsig(project.path())
        .args(["split", "--holdout", "--referee"])
        .arg(empty.path())
        .arg("--out")
        .arg(project.path().join("remaining.json"))
        .assert()
        .failure()
        .code(3);
}

#[test]
fn the_remaining_ids_carry_no_labels_and_resplitting_makes_a_new_holdout() {
    let (project, home) = setup();
    let remaining = project.path().join("remaining.json");
    let out = evalsig(project.path())
        .args(["--format", "json", "split", "--holdout", "--referee"])
        .arg(home.path())
        .arg("--out")
        .arg(&remaining)
        .assert()
        .success();
    let v = json(&out.get_output().stdout);
    let r = &v["results"];
    assert_eq!(v["provenance"]["ledger"], "referee");
    let held = r["holdout"]["items"].as_u64().unwrap();
    assert!(held > 0 && held + r["remaining"]["items"].as_u64().unwrap() == 300);

    let file: Value = serde_json::from_str(&std::fs::read_to_string(&remaining).unwrap()).unwrap();
    let keys: Vec<&str> = file
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["holdout", "items", "schema"]);
    assert!(
        file["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(Value::is_string)
    );
    assert_eq!(file["holdout"], r["holdout"]["hash"]);
    let text = std::fs::read_to_string(&remaining).unwrap();
    assert!(!text.contains("\"record\"") && !text.contains("\"label\""));

    // The line is in the referee ledger, not the agent's.
    let referee_ledger = std::fs::read_to_string(home.path().join("ledger.jsonl")).unwrap();
    let line: Value = serde_json::from_str(referee_ledger.lines().last().unwrap()).unwrap();
    assert_eq!(
        (line["kind"].as_str(), line["ledger"].as_str()),
        (Some("split"), Some("referee"))
    );
    let agent_ledger =
        std::fs::read_to_string(project.path().join(".evalsig/ledger.jsonl")).unwrap();
    assert!(!agent_ledger.contains("\"split\""));

    // Another cut is another holdout, with its own identity and budget.
    let again = evalsig(project.path())
        .args([
            "--format",
            "json",
            "split",
            "--holdout",
            "--seed",
            "99",
            "--referee",
        ])
        .arg(home.path())
        .arg("--out")
        .arg(&remaining)
        .assert()
        .success();
    let second = json(&again.get_output().stdout);
    assert_ne!(second["results"]["holdout"]["hash"], r["holdout"]["hash"]);
    assert_eq!(
        std::fs::read_dir(home.path().join("holdouts"))
            .unwrap()
            .count(),
        2
    );

    // The agent splits within what it was given.
    let w = evalsig(project.path())
        .args(["--format", "json", "split", "--within"])
        .arg(&remaining)
        .assert()
        .success();
    assert_eq!(
        json(&w.get_output().stdout)["results"]["holdout_excluded"],
        true
    );
}
