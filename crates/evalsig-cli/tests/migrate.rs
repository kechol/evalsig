use assert_cmd::Command;
use serde_json::Value;

#[test]
fn current_snapshots_are_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let evalsig = || {
        let mut c = Command::cargo_bin("evalsig").unwrap();
        c.env("EVALSIG_ACTOR", "user:test")
            .arg("--project")
            .arg(dir.path());
        c
    };
    evalsig().arg("init").assert().success();
    evalsig()
        .args([
            "import",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl"),
        ])
        .assert()
        .success();
    let ledger = dir.path().join(".evalsig/ledger.jsonl");
    let before = std::fs::read_to_string(&ledger).unwrap();
    let out = evalsig()
        .args(["--format", "json", "migrate"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["results"]["migrated"], serde_json::json!([]));
    assert_eq!(std::fs::read_to_string(&ledger).unwrap(), before);
}
