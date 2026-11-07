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
    // Half the items moved and the difference is 0.4: the sizing of `power` applies.
    assert_eq!(
        v["next"][0]["command"],
        "evalsig power --delta 0.4 --discordance 0.5"
    );
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

#[test]
fn validation_uses_count_recorded_comparisons_of_the_same_split() {
    let dir = project();
    let config = dir.path().join(".evalsig/config.toml");
    let text = std::fs::read_to_string(&config)
        .unwrap()
        .replace("[compare]\n", "[compare]\nvalidation_warn_after = 2\n");
    std::fs::write(&config, text).unwrap();
    let uses = |v: &Value| v["results"]["validation_uses"].as_u64().unwrap();
    let warned = |v: &Value| {
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "validation_uses")
    };
    let first = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2"]));
    assert_eq!(uses(&first), 1);
    // A scratch comparison reports the count it would have but does not add to it.
    let scratch =
        json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2", "--scratch"]));
    assert_eq!(uses(&scratch), 2);
    let second = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2"]));
    let third = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2"]));
    assert_eq!((uses(&second), uses(&third)), (2, 3));
    assert!(!warned(&second) && warned(&third));
    assert_eq!(first["results"]["split"], third["results"]["split"]);

    // verify recounts from the ledger lines before the saved comparison.
    let saved = dir.path().join(".evalsig/reports").join(format!(
        "{}.json",
        second["provenance"]["output"]
            .as_str()
            .unwrap()
            .strip_prefix("blake3:")
            .unwrap()
    ));
    let v = json(evalsig(dir.path()).arg("verify").arg(&saved));
    assert_eq!(
        v["results"]["match"], true,
        "{}",
        v["results"]["mismatches"]
    );
}

#[test]
fn unpaired_and_sequential_comparisons() {
    let dir = project();
    let v = json(evalsig(dir.path()).args([
        "compare",
        "triage-v1",
        "triage-v2",
        "--unpaired",
        "--scratch",
    ]));
    assert_eq!(v["results"]["paired"], false);
    let m = &v["results"]["metrics"][0];
    // Soft references make the counts weighted: each arm's clusters are resampled.
    assert_eq!(m["interval"], "bootstrap");
    assert!(m["ci"][0].as_f64().unwrap() <= m["delta"].as_f64().unwrap());

    let v = json(evalsig(dir.path()).args([
        "compare",
        "triage-v1",
        "triage-v2",
        "--sequential",
        "--scratch",
    ]));
    let seq = &v["results"]["sequential"];
    assert_eq!(seq["order_seed"], v["provenance"]["seed"]);
    let acc = seq["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric"] == "accuracy")
        .unwrap();
    let (lo, hi) = (
        acc["confidence_sequence"][0].as_f64().unwrap(),
        acc["confidence_sequence"][1].as_f64().unwrap(),
    );
    let d = acc["delta"].as_f64().unwrap();
    assert!(lo <= d && d <= hi);
    // The sequence is wider than the fixed-n interval of the same difference.
    let fixed = &v["results"]["metrics"][0]["ci"];
    assert!(hi - lo > fixed[1].as_f64().unwrap() - fixed[0].as_f64().unwrap());

    evalsig(dir.path())
        .args([
            "compare",
            "triage-v1",
            "triage-v2",
            "--unpaired",
            "--sequential",
        ])
        .assert()
        .failure();
}

#[test]
fn a_rerun_that_moves_more_than_the_difference_is_flagged() {
    let dir = project();
    let codes = |v: &Value| -> Vec<String> {
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["code"].as_str().unwrap().to_owned())
            .collect()
    };
    let v = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2"]));
    assert!(!codes(&v).contains(&"run_variance_exceeds_delta".to_owned()));
    // A second run of prompt-v1 that disagrees with the first on every other item.
    let mut rerun = vec![
        r#"{"record":"run","run_id":"triage-v1-again","system":{"id":"triage","version":"prompt-v1"},"value_semantics":"decision","output_type":"binary","grader":{"id":"rule-check","version":"1","kind":"rule"}}"#.to_owned(),
    ];
    let mut n = 0;
    for line in std::fs::read_to_string(SYNTHETIC).unwrap().lines() {
        let mut r: Value = serde_json::from_str(line).unwrap();
        if r["record"] == "judgment" && r["run_id"] == "triage-v1" {
            r["run_id"] = "triage-v1-again".into();
            if n % 2 == 0
                && let Some(b) = r["value"].as_bool()
            {
                r["value"] = (!b).into();
            }
            n += 1;
            rerun.push(r.to_string());
        }
    }
    // A run of the same version with nothing for this task is not a repeat.
    rerun.push(r#"{"record":"run","run_id":"triage-v1-empty","system":{"id":"triage","version":"prompt-v1"},"value_semantics":"decision","output_type":"binary","grader":{"id":"rule-check","version":"1","kind":"rule"}}"#.to_owned());
    let file = dir.path().join("rerun.jsonl");
    std::fs::write(&file, rerun.join("\n") + "\n").unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    let v = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2"]));
    assert!(
        codes(&v).contains(&"run_variance_exceeds_delta".to_owned()),
        "{}",
        v["warnings"]
    );
    // The configuration switches the warning off.
    let config = dir.path().join(".evalsig/config.toml");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        format!("{text}\n[checks]\nwarn_run_variance_exceeds_delta = false\n"),
    )
    .unwrap();
    let v = json(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2"]));
    assert!(!codes(&v).contains(&"run_variance_exceeds_delta".to_owned()));
}
