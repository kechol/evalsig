use assert_cmd::Command;
use serde_json::Value;

fn evalsig() -> Command {
    Command::cargo_bin("evalsig").unwrap()
}

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl");

#[test]
fn the_ported_examples_validate() {
    let out = evalsig()
        .args(["validate", "--format", "json", "--records", EXAMPLES])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["results"]["records"], 25);
    assert_eq!(v["results"]["invalid_records"], 0);
}

#[test]
fn malformed_records_are_reported_by_line_with_exit_5() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad.jsonl");
    std::fs::write(
        &file,
        concat!(
            "{\"record\":\"item\",\"item_id\":\"a\"}\n",
            "\n",
            "{\"record\":\"item\",\"item_id\":\"b\",\"colour\":\"red\"}\n",
            "{\"record\":\"run\",\"run_id\":\"r\",\"system\":{\"id\":\"s\",\"version\":\"1\"},\"value_semantics\":\"score\"}\n",
        ),
    )
    .unwrap();
    let out = evalsig()
        .args(["validate", "--format", "json", "--records"])
        .arg(&file)
        .assert()
        .code(5)
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["verdict"]["code"], "blocked");
    assert_eq!(v["results"]["invalid_records"], 2);
    let issues = v["results"]["issues"].as_array().unwrap();
    assert_eq!(issues[0]["line"], 3);
    assert_eq!(issues[0]["code"], "schema.additional_properties");
    assert!(
        issues
            .iter()
            .any(|i| i["line"] == 4 && i["code"] == "schema.required")
    );
}

#[test]
fn schema_records_prints_the_schema() {
    let out = evalsig()
        .args(["schema", "records", "--raw"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["$id"], "urn:evalsig:schema:records:1");
    let out = evalsig()
        .args(["schema", "ledger-entry", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["results"]["id"], "urn:evalsig:schema:ledger-entry:1");
    evalsig().args(["schema", "nope"]).assert().code(64);
}
