//! `import` refuses oversized and deeply nested input before parsing it.

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

fn refused(c: &mut Command) -> Value {
    let out = c.assert().code(3).get_output().stderr.clone();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn oversized_and_deeply_nested_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let records = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl");
    let v = refused(evalsig(dir.path()).args(["import", "--max-bytes", "1K", records]));
    assert_eq!(v["error"]["code"], "input_limit");

    let deep = dir.path().join("deep.jsonl");
    std::fs::write(
        &deep,
        format!("{}{}\n", "[".repeat(1_000_000), "]".repeat(1_000_000)),
    )
    .unwrap();
    let v = refused(evalsig(dir.path()).arg("import").arg(&deep));
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nested deeper than 64")
    );

    evalsig(dir.path())
        .args(["import", "--max-bytes", "1M", records])
        .assert()
        .success();
    evalsig(dir.path())
        .args(["import", "--max-bytes", "12X", records])
        .assert()
        .code(64);
}
