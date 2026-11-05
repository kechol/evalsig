//! `evalsig map probe` shows shapes, not contents.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn probe(file: &Path, extra: &[&str]) -> Value {
    let out = Command::cargo_bin("evalsig")
        .unwrap()
        .args(["--format", "json", "map", "probe"])
        .arg(file)
        .args(extra)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(rel)
}

#[test]
fn secret_like_and_free_text_values_are_never_printed() {
    let file = fixture("probe/secret-like.jsonl");
    for text in [
        String::from_utf8(
            Command::cargo_bin("evalsig")
                .unwrap()
                .args(["map", "probe"])
                .arg(&file)
                .assert()
                .success()
                .get_output()
                .stdout
                .clone(),
        )
        .unwrap(),
        probe(&file, &[]).to_string(),
    ] {
        assert!(
            !text.contains("Maria") && !text.contains("sk-aaa"),
            "{text}"
        );
    }
    let v = probe(&file, &[]);
    let paths = v["results"]["probe"]["paths"].as_array().unwrap();
    let by = |p: &str| paths.iter().find(|s| s["path"] == p).unwrap();
    assert_eq!(by("$.comment")["values_withheld"], "free text");
    assert!(by("$.comment")["max_len"].as_u64().unwrap() > 32);
    assert_eq!(by("$.api_key")["values_withheld"], "secret-like");
    assert_eq!(
        by("$.verdict")["values"],
        serde_json::json!(["approve", "reject"])
    );
    assert_eq!(
        v["results"]["probe"]["key_candidates"],
        serde_json::json!(["$.ticket"])
    );
    assert_eq!(
        v["results"]["probe"]["replicate_candidates"],
        serde_json::json!(["$.attempt"])
    );
    let quiet = probe(&file, &["--no-values"]);
    assert!(!quiet.to_string().contains("approve"));
}

#[test]
fn harness_outputs_are_recognised() {
    for (rel, format) in [
        ("harness/inspect/probe.eval", "inspect"),
        ("harness/inspect/probe.json", "inspect"),
        ("harness/promptfoo/out.json", "promptfoo"),
    ] {
        let v = probe(&fixture(rel), &[]);
        assert_eq!(v["results"]["probe"]["known"]["format"], format, "{rel}");
        assert!(
            v["next"][0]["command"]
                .as_str()
                .unwrap()
                .starts_with("evalsig import --adapter")
        );
    }
}

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

#[test]
fn a_draft_leaves_every_decision_and_decide_fills_them() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let log = fixture("probe/secret-like.jsonl");
    let mapping = dir.path().join("mapping.toml");
    let out = evalsig(dir.path())
        .args(["map", "draft"])
        .arg(&log)
        .arg("--out")
        .arg(&mapping)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["verdict"]["code"], "needs_decisions");
    let text = std::fs::read_to_string(&mapping).unwrap();
    assert_eq!(text.matches("DECIDE").count(), 6, "{text}");
    assert!(!text.contains("Maria") && !text.contains("sk-aaa"));
    let import = |c: &mut Command| {
        c.args(["import", "--mapping"])
            .arg(&mapping)
            .args(["--arg", "run-id=r1"])
            .arg(&log);
    };
    let mut c = evalsig(dir.path());
    import(&mut c);
    c.assert().code(5);

    for (id, choice) in [
        ("item_key", "$.ticket"),
        ("value_semantics", "decision"),
        ("value", "$.verdict"),
        ("output_type", "binary"),
        ("value.none", r#"{"approve": true, "reject": false}"#),
        ("replicate_kind", "gen_replicate=$.attempt"),
        ("missing.reason", r#"{"timeout": "timeout"}"#),
    ] {
        evalsig(dir.path())
            .args([
                "map",
                "decide",
                id,
                choice,
                "--by",
                "user:alice",
                "--mapping",
            ])
            .arg(&mapping)
            .assert()
            .success();
    }
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[project]\nname = \"x\"\n",
    )
    .unwrap();
    let mut c = evalsig(dir.path());
    import(&mut c);
    let v: Value = serde_json::from_slice(&c.assert().success().get_output().stdout).unwrap();
    assert_eq!(v["results"]["records"]["judgments"], 8);
    let ledger = std::fs::read_to_string(dir.path().join(".evalsig/ledger.jsonl")).unwrap();
    assert_eq!(ledger.matches("\"mapping_decision\"").count(), 7);
    evalsig(dir.path())
        .args([
            "map",
            "decide",
            "nope",
            "x",
            "--by",
            "user:alice",
            "--mapping",
        ])
        .arg(&mapping)
        .assert()
        .code(64);
}

fn map_test(mapping: &Path, file: &Path, code: i32) -> Value {
    let out = Command::cargo_bin("evalsig")
        .unwrap()
        .args(["--format", "json", "map", "test"])
        .arg(mapping)
        .arg(file)
        .args(["--arg", "run-id=triage-v1", "--arg"])
        .arg(format!(
            "labels={}",
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/custom-harness/labels.sample.json")
                .display()
        ))
        .assert()
        .code(code)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

fn example(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(rel)
}

#[test]
fn shape_shifted_records_give_fixed_counts() {
    let dir = tempfile::tempdir().unwrap();
    let rows: Vec<Value> = std::fs::read_to_string(example("custom-harness/run-log.sample.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let write = |name: &str, rs: Vec<Value>| {
        let path = dir.path().join(name);
        let text: Vec<String> = rs.iter().map(|r| r.to_string()).collect();
        std::fs::write(&path, text.join("\n")).unwrap();
        path
    };
    let deeper = write(
        "deeper.jsonl",
        rows.iter()
            .map(|r| {
                let mut r = r.clone();
                let runs = r.as_object_mut().unwrap().remove("runs").unwrap();
                r["result"] = serde_json::json!({ "runs": runs });
                r
            })
            .collect(),
    );
    let array = write(
        "array.jsonl",
        rows.iter()
            .map(|r| {
                let mut r = r.clone();
                let runs: Vec<Value> = r["runs"].as_object().unwrap().values().cloned().collect();
                r["runs"] = Value::Array(runs);
                r
            })
            .collect(),
    );
    let moved = write(
        "moved.jsonl",
        rows.iter()
            .map(|r| {
                let mut r = r.clone();
                if let Some(Value::Object(m)) = r.as_object_mut().unwrap().remove("missing") {
                    for (k, v) in m {
                        r["runs"][k] = serde_json::json!({ "error": v });
                    }
                }
                r
            })
            .collect(),
    );
    let duplicated = write(
        "duplicated.jsonl",
        rows.iter().chain(&rows[..1]).cloned().collect(),
    );
    let mapping = example("mapping.toml");
    let counts = |v: &Value| {
        let r = &v["results"];
        (
            r["coverage"]["mapped"].as_u64().unwrap(),
            r["integrity"]["duplicate_judgments"].as_u64().unwrap(),
            r["values"]["missing"]["invalid_output"]
                .as_u64()
                .unwrap_or(0),
            r["unused_paths"].clone(),
        )
    };
    assert_eq!(
        counts(&map_test(&mapping, &deeper, 0)),
        (
            3,
            0,
            5,
            serde_json::json!(["$.result.runs.a.level", "$.result.runs.b.level"])
        )
    );
    assert_eq!(
        counts(&map_test(&mapping, &array, 0)),
        (3, 0, 5, serde_json::json!(["$.runs[*].level"]))
    );
    assert_eq!(
        counts(&map_test(&mapping, &moved, 0)),
        (3, 0, 1, serde_json::json!(["$.runs.b.error"]))
    );
    assert_eq!(
        counts(&map_test(&mapping, &duplicated, 0)),
        (4, 2, 0, serde_json::json!([]))
    );
}

#[test]
fn test_expectations_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let mapping = dir.path().join("mapping.toml");
    std::fs::copy(example("mapping.toml"), &mapping).unwrap();
    let log = example("custom-harness/run-log.sample.jsonl");
    let row: Value = serde_json::from_str(
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .nth(1)
            .unwrap(),
    )
    .unwrap();
    // The converter script's records for this row are the expectation.
    let expected: Vec<Value> = std::fs::read_to_string(example("custom-harness/records.out.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|r| r["record"] == "run" || r["item_id"] == "priority:t-0002")
        .collect();
    let case = serde_json::json!({"input": row, "records": expected});
    std::fs::write(dir.path().join("mapping.tests.jsonl"), format!("{case}\n")).unwrap();
    let v = map_test(&mapping, &log, 0);
    assert_eq!(v["results"]["tests"]["passed"], 1);
    let mut wrong = case.clone();
    wrong["records"][0]["run_id"] = Value::String("other".into());
    std::fs::write(dir.path().join("mapping.tests.jsonl"), format!("{wrong}\n")).unwrap();
    let v = map_test(&mapping, &log, 1);
    assert_eq!(v["verdict"]["code"], "fail");
}

#[test]
fn capabilities_name_what_the_data_lack() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    evalsig(dir.path())
        .arg("import")
        .arg(example("custom-harness/records.out.jsonl"))
        .assert()
        .success();
    let out = evalsig(dir.path())
        .args(["capabilities", "--run", "triage-v1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let c = &v["results"]["capabilities"];
    let available: Vec<&str> = c["available"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert!(available.contains(&"hit_rate") && available.contains(&"replicate_disagreement"));
    let missing: Vec<&str> = c["unavailable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["missing"].as_str().unwrap())
        .collect();
    assert!(missing.contains(&"input_ref") && missing.contains(&"occurred_at"));
    evalsig(dir.path())
        .args(["capabilities", "--run", "nope"])
        .assert()
        .code(64);
}

#[test]
fn drafts_read_rows_of_a_json_document_and_refuse_archives() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("results.json");
    let rows: Vec<Value> = (0..5)
        .map(|i| serde_json::json!({"id": format!("q{i}"), "verdict": i % 2 == 0}))
        .collect();
    std::fs::write(&doc, serde_json::json!({"results": rows}).to_string()).unwrap();
    let out = evalsig(dir.path())
        .args(["map", "draft"])
        .arg(&doc)
        .arg("--out")
        .arg(dir.path().join("m.toml"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(
        v["results"]["candidates"]["item_key"],
        serde_json::json!(["$.id"])
    );
    let text = std::fs::read_to_string(dir.path().join("m.toml")).unwrap();
    assert!(text.contains("records_path = \"$.results[*]\""), "{text}");
    evalsig(dir.path())
        .args(["map", "draft"])
        .arg(fixture("harness/inspect/probe.eval"))
        .arg("--out")
        .arg(dir.path().join("n.toml"))
        .assert()
        .code(64);
}

#[test]
fn deciding_again_keeps_earlier_choices_consistent() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let mapping = dir.path().join("mapping.toml");
    evalsig(dir.path())
        .args(["map", "draft"])
        .arg(fixture("probe/secret-like.jsonl"))
        .arg("--out")
        .arg(&mapping)
        .assert()
        .success();
    let decide = |id: &str, choice: &str, code: i32| {
        evalsig(dir.path())
            .args([
                "map",
                "decide",
                id,
                choice,
                "--by",
                "user:alice",
                "--mapping",
            ])
            .arg(&mapping)
            .assert()
            .code(code);
    };
    decide("value", "$.verdict", 0);
    decide("value.none", r#"{"approve": true, "reject": false}"#, 0);
    decide("value", "$.verdict", 0);
    decide("output_type", "ordinal:3", 0);
    decide("output_type", "binary", 0);
    decide("replicate_kind", "none", 0);
    decide("positive", r#"{"positive": [1]}"#, 64);
    let text = std::fs::read_to_string(&mapping).unwrap();
    assert!(text.contains("value_map"), "{text}");
    assert!(!text.contains("levels"), "{text}");
    assert!(!text.contains("gen_replicate"), "{text}");
}

#[test]
fn a_mapping_draft_is_never_written_into_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut init = assert_cmd::Command::cargo_bin("evalsig").unwrap();
    init.arg("--project")
        .arg(dir.path())
        .arg("init")
        .assert()
        .success();
    let config = dir.path().join(".evalsig/config.toml");
    let before = std::fs::read_to_string(&config).unwrap();
    let src = dir.path().join("rows.jsonl");
    std::fs::write(
        &src,
        "{\"id\": 1, \"ok\": true}\n{\"id\": 2, \"ok\": false}\n",
    )
    .unwrap();
    for target in [config.clone(), dir.path().join(".evalsig/ledger.jsonl")] {
        let out = assert_cmd::Command::cargo_bin("evalsig")
            .unwrap()
            .args(["map", "draft"])
            .arg(&src)
            .arg("--out")
            .arg(&target)
            .arg("--force")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64), "{}", target.display());
    }
    // A link outside the store that points into it is refused too.
    #[cfg(unix)]
    {
        let link = dir.path().join("mapping.toml");
        std::os::unix::fs::symlink(&config, &link).unwrap();
        let out = assert_cmd::Command::cargo_bin("evalsig")
            .unwrap()
            .args(["map", "draft"])
            .arg(&src)
            .arg("--out")
            .arg(&link)
            .arg("--force")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64));
    }
    assert_eq!(std::fs::read_to_string(&config).unwrap(), before);
}

/// The Langfuse example (T-122): judge and annotation scores on the same traces become a
/// run's judgments and human labels, and the expectations beside the mapping hold.
#[test]
fn the_langfuse_example_maps_judge_and_annotation_scores() {
    let mapping = example("langfuse/mapping.toml");
    let traces = example("langfuse/traces.jsonl");
    let out = Command::cargo_bin("evalsig")
        .unwrap()
        .args(["--format", "json", "map", "test"])
        .arg(&mapping)
        .arg(&traces)
        .args(["--arg", "run-id=judge-v1"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let r = &v["results"];
    assert_eq!(r["tests"]["passed"], 1, "{r}");
    assert_eq!(r["coverage"]["mapped"], 12, "{r}");
    assert_eq!(r["integrity"]["items_without_label"], 0, "{r}");
    assert_eq!(r["invalid_records"], 0, "{r}");

    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    evalsig(dir.path())
        .args(["import", "--mapping"])
        .arg(&mapping)
        .args(["--arg", "run-id=judge-v1"])
        .arg(&traces)
        .assert()
        .success();
    let out = evalsig(dir.path())
        .args(["report", "judge-v1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    // The judge agrees with the reviewers on 9 of the 12 traces.
    let accuracy = v["results"]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric"] == "accuracy")
        .unwrap();
    assert_eq!(accuracy["value"], 0.75, "{accuracy}");
    assert_eq!(v["results"]["reference"]["raters"], 2);
}
