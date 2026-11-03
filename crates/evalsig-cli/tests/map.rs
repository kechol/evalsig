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
