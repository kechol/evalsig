//! Importing Inspect logs: `.eval` archives, `.json` logs and `[[import]]` blocks.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/harness/inspect")
        .join(name)
}

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

fn json(c: &mut Command) -> Value {
    serde_json::from_slice(&c.assert().success().get_output().stdout).unwrap()
}

#[test]
fn eval_and_json_logs_import_and_report() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let v = json(evalsig(dir.path()).arg("import").arg(fixture("probe.eval")));
    assert_eq!(
        v["results"]["records"],
        serde_json::json!({"runs": 1, "items": 2, "judgments": 4, "contents": 5})
    );
    let w = json(
        evalsig(dir.path())
            .args(["import", "--adapter", "inspect"])
            .arg(fixture("probe.json")),
    );
    assert_eq!(w["results"]["records"]["judgments"], 4);
    let runs = json(evalsig(dir.path()).arg("runs"));
    let run = runs["results"]["runs"][0]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let report = json(evalsig(dir.path()).args(["report", &run]));
    let mean = report["results"]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric"] == "mean_score")
        .unwrap();
    // s1 scores I and s2 scores C in both epochs.
    assert_eq!(mean["value"], 0.5);
}

#[test]
fn an_inspect_import_block_applies_its_value_map() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::create_dir_all(dir.path().join("logs")).unwrap();
    std::fs::copy(fixture("probe.eval"), dir.path().join("logs/probe.eval")).unwrap();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[[import]]\nname = \"inspect-logs\"\nadapter = \"inspect\"\npath = \"logs/*.eval\"\nscore = \"includes\"\nvalue_map = { C = 1, I = 0.25 }\n",
    )
    .unwrap();
    let v = json(evalsig(dir.path()).args(["import", "--from", "inspect-logs"]));
    assert_eq!(v["results"]["records"]["judgments"], 4);
    let runs = json(evalsig(dir.path()).arg("runs"));
    let run = runs["results"]["runs"][0]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let report = json(evalsig(dir.path()).args(["report", &run]));
    let mean = report["results"]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric"] == "mean_score")
        .unwrap();
    assert_eq!(mean["value"], 0.625);
}
