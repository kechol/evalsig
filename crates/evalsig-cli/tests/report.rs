use std::path::Path;

mod common;

use assert_cmd::Command;
use serde_json::Value;

const RECORDS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl");
const GOLDEN: &str = include_str!("snapshots/report-r-base.json");

pub const CONFIG: &str = "[project]\nname = \"example\"\npositive = { ordinal_at_least = 2 }\ndecision = { ordinal_at_least = 2 }\n[stats]\nseed = 20261029\n";

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    evalsig(dir.path())
        .args(["import", RECORDS])
        .assert()
        .success();
    dir
}

fn json(c: &mut Command) -> Value {
    let v = serde_json::from_slice(&c.assert().success().get_output().stdout).unwrap();
    common::check_results(&v);
    v
}

/// Results and the deterministic provenance; the tool version is normalized.
fn comparable(mut v: Value) -> Value {
    v["provenance"]["tool"] = Value::String("evalsig <VERSION>".into());
    v
}

#[test]
fn the_examples_produce_a_fixed_report() {
    let dir = project();
    let v = comparable(json(evalsig(dir.path()).args(["report", "r-base"])));
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(v, golden, "{}", serde_json::to_string_pretty(&v).unwrap());
}

#[test]
fn reports_are_saved_and_recorded() {
    let dir = project();
    let v = json(evalsig(dir.path()).args(["report", "r-base"]));
    let ledger = std::fs::read_to_string(dir.path().join(".evalsig/ledger.jsonl")).unwrap();
    let last: Value = serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "report");
    assert_eq!(last["summary"]["run"], "r-base");
    assert_eq!(last["inputs"][1], v["provenance"]["config"]);
    let hex = last["outputs"][0]
        .as_str()
        .unwrap()
        .strip_prefix("blake3:")
        .unwrap()
        .to_owned();
    assert!(
        dir.path()
            .join(".evalsig/reports")
            .join(format!("{hex}.json"))
            .is_file()
    );
}

#[test]
fn options_shape_the_report() {
    let dir = project();
    let by = json(evalsig(dir.path()).args(["report", "r-base", "--by", "lang", "--items"]));
    assert!(by["results"]["by"]["ja"].is_array());
    assert!(by["results"]["by"]["en"].is_array());
    assert_eq!(by["results"]["items"].as_array().unwrap().len(), 2);
    // Before alice's revision (2026-09-10) and bob's and carol's labels, q-0001 had
    // only her first label.
    let early = json(evalsig(dir.path()).args([
        "report",
        "r-base",
        "--labels-as-of",
        "2026-08-03T12:00:00Z",
    ]));
    assert_eq!(early["results"]["reference"]["raters"], 1);
    let score = json(evalsig(dir.path()).args(["report", "r-judge-score", "--threshold", "0.5"]));
    let metrics: Vec<&str> = score["results"]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["metric"].as_str().unwrap())
        .collect();
    assert_eq!(metrics, ["mean_score", "rate_at_least"]);
    assert_eq!(score["results"]["metrics"][1]["value"], 1.0);
    let mean = json(evalsig(dir.path()).args(["report", "r-judge-score"]));
    assert_eq!(mean["results"]["metrics"][0]["metric"], "mean_score");
    assert_eq!(mean["results"]["metrics"][0]["value"], 0.71);
    evalsig(dir.path())
        .args(["report", "nope"])
        .assert()
        .code(64);
    evalsig(dir.path())
        .args(["report", "r-base", "--estimand", "best@2"])
        .assert()
        .code(64);
}

#[test]
fn reference_agreement_strata_split_the_synthetic_items() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        include_str!("../../../examples/synthetic/config.toml"),
    )
    .unwrap();
    let records = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/synthetic/records.jsonl"
    );
    evalsig(dir.path())
        .args(["import", records])
        .assert()
        .success();
    let v = json(evalsig(dir.path()).args(["report", "triage-v1", "--by", "reference_agreement"]));
    let by = v["results"]["by"].as_object().unwrap();
    assert_eq!(by.keys().collect::<Vec<_>>(), ["split", "unanimous"]);
    let n = |stratum: &str| {
        by[stratum]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["metric"] == "accuracy")
            .unwrap()["n"]
            .as_f64()
            .unwrap()
    };
    // 75 of the 300 synthetic items have two human raters; only they are stratified.
    assert_eq!(n("split") + n("unanimous"), 75.0);
    assert!(n("split") > 0.0 && n("unanimous") > 0.0);
}
