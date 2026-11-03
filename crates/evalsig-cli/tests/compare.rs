use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");
const EXAMPLE_OUTPUT: &str = include_str!("../../../examples/agent/compare-output.json");

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--project"])
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

fn json(c: &mut Command) -> Value {
    serde_json::from_slice(
        &c.arg("--format")
            .arg("json")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .unwrap()
}

#[test]
fn the_published_compare_example_is_reproduced() {
    let dir = project();
    let mut v = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2"]));
    v["provenance"]["tool"] = Value::String("evalsig <VERSION>".into());
    let example: Value = serde_json::from_str(EXAMPLE_OUTPUT).unwrap();
    assert_eq!(v, example, "{}", serde_json::to_string_pretty(&v).unwrap());
    assert_eq!(v["verdict"]["code"], "improved");
    let codes: Vec<&str> = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["missing_rate_differs"]);
}

#[test]
fn compare_is_recorded_unless_scratch() {
    let dir = project();
    let ledger = dir.path().join(".evalsig/ledger.jsonl");
    json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2"]));
    let lines = std::fs::read_to_string(&ledger).unwrap();
    let last: Value = serde_json::from_str(lines.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "compare");
    assert_eq!(last["summary"]["verdict"], "improved");
    let before = lines.lines().count();
    let v = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2", "--scratch"]));
    assert!(v["provenance"]["ledger_seq"].is_null());
    assert_eq!(
        std::fs::read_to_string(&ledger).unwrap().lines().count(),
        before
    );
}

#[test]
fn items_print_as_jsonl() {
    let dir = project();
    let out = evalsig(dir.path())
        .args([
            "--format",
            "jsonl",
            "compare",
            "triage-v1",
            "triage-v2",
            "--items",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 300);
    assert!(rows[0]["item_id"].is_string() && rows[0]["candidate"].is_number());
    evalsig(dir.path())
        .args(["--format", "jsonl", "compare", "triage-v1", "triage-v2"])
        .assert()
        .code(64);
    evalsig(dir.path())
        .args(["--format", "jsonl", "runs"])
        .assert()
        .code(64);
}

#[test]
fn several_candidates_are_holm_adjusted() {
    let dir = project();
    let v = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2", "triage-v2"]));
    assert_eq!(v["results"]["comparisons"].as_array().unwrap().len(), 2);
    assert_eq!(
        v["results"]["holm"]["adjusted_p"].as_array().unwrap().len(),
        2
    );
    assert_eq!(v["results"]["selected_after_comparison"], true);
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "selected_after_comparison")
    );
}

#[test]
fn too_few_units_give_insufficient_data() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[project]\npositive = { ordinal_at_least = 2 }\ndecision = { ordinal_at_least = 2 }\n",
    )
    .unwrap();
    evalsig(dir.path())
        .args([
            "import",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl"),
        ])
        .assert()
        .success();
    let v = json(evalsig(dir.path()).args(["compare", "r-base", "r-cand"]));
    assert_eq!(v["verdict"]["code"], "insufficient_data");
    assert_eq!(v["results"]["metrics"][0]["interval"], "too_few_units");
    assert!(
        v["next"][0]["command"]
            .as_str()
            .unwrap()
            .starts_with("evalsig power")
    );
}
