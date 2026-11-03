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
    assert_eq!(v["next"], serde_json::json!([]));
}

#[test]
fn jsonl_rows_cover_every_candidate() {
    let dir = project();
    let out = evalsig(dir.path())
        .args([
            "--format",
            "jsonl",
            "compare",
            "triage-v1",
            "triage-v2",
            "triage-v2",
            "--items",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(out).unwrap().lines().count(), 600);
}

#[test]
fn arms_with_different_output_types_are_refused() {
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
    let out = evalsig(dir.path())
        .args(["--format", "json", "compare", "r-base", "r-judge-score"])
        .assert()
        .code(3)
        .get_output()
        .stderr
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["code"], "incomparable");
}

#[test]
fn preference_records_add_the_win_rate_and_position_checks() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[project]\nname = \"example\"\npositive = { ordinal_at_least = 2 }\ndecision = { ordinal_at_least = 2 }\n",
    )
    .unwrap();
    let examples = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl");
    evalsig(dir.path())
        .args(["import", examples])
        .assert()
        .success();
    let v = json(evalsig(dir.path()).args(["compare", "r-base", "r-cand", "--scratch"]));
    let p = &v["results"]["preference"];
    // q-0001 is judged in both orders and the right side wins each time (½ for the
    // candidate after the swap cancels); q-0002 is a tie.
    assert_eq!(p["win_rate"]["value"], 0.5);
    assert_eq!(p["position_bias"]["swapped_pairs"], 1);
    assert_eq!(
        p["raters"],
        serde_json::json!(["model:pairwise-judge", "user:bob"])
    );

    // Judgments in one order only: no swapped pair to measure position bias with.
    let one_order = dir.path().join("one-order.jsonl");
    std::fs::write(
        &one_order,
        r#"{"record":"preference","item_id":"q-0003","left_run_id":"r-base","right_run_id":"r-cand","rater_id":"user:carol","source":"human","outcome":"left"}
"#,
    )
    .unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    evalsig(dir2.path()).arg("init").assert().success();
    std::fs::copy(
        dir.path().join(".evalsig/config.toml"),
        dir2.path().join(".evalsig/config.toml"),
    )
    .unwrap();
    let records: String = std::fs::read_to_string(examples)
        .unwrap()
        .lines()
        .filter(|l| !l.contains("\"preference\""))
        .map(|l| format!("{l}\n"))
        .collect();
    let without = dir2.path().join("records.jsonl");
    std::fs::write(&without, records).unwrap();
    for f in [&without, &one_order] {
        evalsig(dir2.path()).arg("import").arg(f).assert().success();
    }
    let v = json(evalsig(dir2.path()).args(["compare", "r-base", "r-cand", "--scratch"]));
    let codes: Vec<&str> = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["code"].as_str())
        .collect();
    assert!(codes.contains(&"no_swapped_pairs"), "{codes:?}");
    assert_eq!(v["results"]["preference"]["win_rate"]["value"], 0.0);
}
