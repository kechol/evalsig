//! `evalsig check` (D-021): exit codes for passing, failing and insufficient
//! conditions, and the JUnit output against a golden file.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");
const GOLDEN: &str = include_str!("snapshots/check.junit.xml");

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .arg("--project")
        .arg(dir);
    c
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

/// Accuracy (0.758 on 300 items) and the system-side missing rate (6 of 300).
fn conditions(accuracy_at_least: f64, min_items: u64) -> String {
    format!(
        r#"name = "triage stays usable"
min_items = {min_items}

[[conditions]]
metric = "accuracy"
rule = "lower_bound_at_least"
value = {accuracy_at_least}
sided = "one"

[[conditions]]
metric = "missing_rate"
rule = "upper_bound_at_most"
value = 0.05
sided = "one"
"#
    )
}

fn check(dir: &Path, toml: &str, format: &str) -> (i32, String) {
    let file = dir.join("check.toml");
    std::fs::write(&file, toml).unwrap();
    let out = evalsig(dir)
        .args(["--format", format, "check"])
        .arg(&file)
        .args(["--run", "triage-v1"])
        .output()
        .unwrap();
    let text = String::from_utf8(if out.stdout.is_empty() {
        out.stderr
    } else {
        out.stdout
    })
    .unwrap();
    (out.status.code().unwrap(), text)
}

#[test]
fn exit_codes_follow_the_conditions() {
    let dir = project();
    let (code, text) = check(dir.path(), &conditions(0.7, 100), "json");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(code, 0, "{text}");
    assert_eq!(v["verdict"]["code"], "pass");
    assert_eq!(v["results"]["preregistered"], false);
    assert!(
        v["verdict"]["text"]
            .as_str()
            .unwrap()
            .contains("non-preregistered")
    );
    // The one-sided bound is the lower end of the two-sided 90% interval.
    let c = &v["results"]["conditions"][0];
    assert!(c["bound"].as_f64().unwrap() > 0.7 && c["bound"].as_f64().unwrap() < 0.758);
    assert_eq!(v["results"]["conditions"][1]["estimate"], 0.02);

    let (code, text) = check(dir.path(), &conditions(0.8, 100), "json");
    assert_eq!(code, 1, "{text}");
    let (code, text) = check(dir.path(), &conditions(0.7, 1000), "json");
    assert_eq!(code, 4, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["verdict"]["code"], "insufficient_data");

    for refused in [
        format!("hypothesis = \"x\"\n{}", conditions(0.7, 100)),
        conditions(0.7, 100).replace("lower_bound_at_least", "not_worse_than"),
        conditions(0.7, 100).replace("metric = \"accuracy\"", "metric = \"auc\""),
    ] {
        let (code, text) = check(dir.path(), &refused, "json");
        assert_eq!(code, 3, "{text}");
    }
}

#[test]
fn junit_has_one_case_per_condition() {
    let dir = project();
    let (code, xml) = check(dir.path(), &conditions(0.8, 100), "junit");
    assert_eq!(code, 1);
    assert_eq!(xml, GOLDEN);
}
