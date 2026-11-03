use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

fn json(out: &[u8]) -> Value {
    serde_json::from_slice(out).unwrap()
}

fn project_with(lines: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let f = dir.path().join("r.jsonl");
    std::fs::write(&f, lines).unwrap();
    evalsig(dir.path()).arg("import").arg(&f).assert().success();
    dir
}

const RUN: &str =
    r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#;

#[test]
fn list_checks_publishes_the_registry() {
    let dir = tempfile::tempdir().unwrap();
    let v = json(
        &evalsig(dir.path())
            .args(["validate", "--list-checks"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let checks = v["results"]["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 23);
    let mixed = checks.iter().find(|c| c["code"] == "mixed_grader").unwrap();
    assert_eq!(mixed["default"], "stop");
    assert_eq!(mixed["override_flag"], "--allow-mixed-grader");
}

#[test]
fn a_stopping_check_exits_5_and_its_flag_lifts_it() {
    let lines = format!(
        "{RUN}\n{}\n",
        r#"{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"binary","missing":{"reason":"other"}}"#
    );
    let dir = project_with(&lines);
    let v = json(
        &evalsig(dir.path())
            .arg("validate")
            .assert()
            .code(5)
            .get_output()
            .stdout,
    );
    assert_eq!(v["verdict"]["code"], "blocked");
    assert_eq!(v["results"]["n_missing"], 1);
    assert_eq!(v["results"]["missing_by_reason"]["other"], 1);
    assert!(
        v["warnings"][0]["message"]
            .as_str()
            .unwrap()
            .contains("--allow-unexplained-missing")
    );
    let v = json(
        &evalsig(dir.path())
            .args(["validate", "--allow-unexplained-missing"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(v["warnings"][0]["code"], "unexplained_missing");
}

#[test]
fn clean_data_passes_and_still_reports_missing_counts() {
    let lines = format!(
        "{RUN}\n{}\n{}\n",
        r#"{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"binary","value":true}"#,
        r#"{"record":"judgment","item_id":"b","run_id":"r","gen_replicate":0,"output_type":"binary","missing":{"reason":"timeout"}}"#
    );
    let dir = project_with(&lines);
    let v = json(
        &evalsig(dir.path())
            .arg("validate")
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(v["results"]["values"], 1);
    assert_eq!(v["results"]["n_missing"], 1);
    assert_eq!(v["warnings"], serde_json::json!([]));
}

#[test]
fn the_example_data_triggers_the_model_reference_warning() {
    let dir = project_with(include_str!("../../../examples/records.jsonl"));
    let v = json(
        &evalsig(dir.path())
            .arg("validate")
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let codes: Vec<&str> = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"model_derived_reference"), "{codes:?}");
}

#[test]
fn records_mode_rejects_flags_it_would_ignore() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("r.jsonl");
    std::fs::write(&f, format!("{RUN}\n")).unwrap();
    for flag in [
        "--list-checks",
        "--fail-on-secret",
        "--allow-unexplained-missing",
    ] {
        evalsig(dir.path())
            .args(["validate", "--records"])
            .arg(&f)
            .arg(flag)
            .assert()
            .code(64);
    }
}

#[test]
fn label_missing_values_are_counted_too() {
    let lines = format!(
        "{RUN}\n{}\n",
        r#"{"record":"label","item_id":"a","rater_id":"u","source":"human","version":1,"missing":{"reason":"not_run"}}"#
    );
    let dir = project_with(&lines);
    let v = json(
        &evalsig(dir.path())
            .arg("validate")
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(v["results"]["n_missing"], 1);
    assert_eq!(v["results"]["missing_by_reason"]["not_run"], 1);
}
