use std::path::{Path, PathBuf};

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
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

fn json_of(c: &mut Command, code: i32) -> Value {
    serde_json::from_slice(&c.assert().code(code).get_output().stdout).unwrap()
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

fn saved(dir: &Path, v: &Value) -> PathBuf {
    let hex = v["provenance"]["output"]
        .as_str()
        .unwrap()
        .strip_prefix("blake3:")
        .unwrap()
        .to_owned();
    dir.join(".evalsig/reports").join(format!("{hex}.json"))
}

#[test]
fn a_report_and_a_comparison_verify() {
    let dir = project();
    for cmd in [
        vec!["report", "triage-v1", "--by", "channel"],
        vec!["compare", "triage-v1", "triage-v2"],
    ] {
        let v = json_of(evalsig(dir.path()).args(&cmd), 0);
        let file = saved(dir.path(), &v);
        let r = json_of(evalsig(dir.path()).arg("verify").arg(&file), 0);
        assert_eq!(r["results"]["match"], true, "{cmd:?}: {r}");
        assert_eq!(r["results"]["recomputed_output"], v["provenance"]["output"]);
    }
}

#[test]
fn verification_uses_the_recorded_data_not_later_imports() {
    let dir = project();
    let v = json_of(evalsig(dir.path()).args(["report", "triage-v1"]), 0);
    let later = dir.path().join("later.jsonl");
    std::fs::write(&later, "{\"record\":\"judgment\",\"item_id\":\"t-0000\",\"run_id\":\"triage-v1\",\"gen_replicate\":0,\"output_type\":\"binary\",\"value\":false}\n").unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&later)
        .assert()
        .success();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[stats]\nseed = 1\n",
    )
    .unwrap();
    let r = json_of(
        evalsig(dir.path()).arg("verify").arg(saved(dir.path(), &v)),
        0,
    );
    assert_eq!(r["results"]["match"], true);
}

#[test]
fn a_hand_edited_value_is_reported() {
    let dir = project();
    let v = json_of(evalsig(dir.path()).args(["report", "triage-v1"]), 0);
    let file = saved(dir.path(), &v);
    let mut edited: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    edited["results"]["metrics"][0]["value"] = Value::from(0.99);
    let out = dir.path().join("edited.json");
    std::fs::write(&out, serde_json::to_string(&edited).unwrap()).unwrap();
    let r = json_of(evalsig(dir.path()).arg("verify").arg(&out), 1);
    assert_eq!(r["results"]["match"], false);
    assert_eq!(r["verdict"]["code"], "fail");
    let m = &r["results"]["mismatches"][0];
    assert_eq!(m["path"], "/metrics/accuracy/value");
    assert_eq!(m["saved"], 0.99);
    let ledger = std::fs::read_to_string(dir.path().join(".evalsig/ledger.jsonl")).unwrap();
    let last: Value = serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "verify");
    assert_eq!(last["summary"]["match"], false);
}
