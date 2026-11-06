use std::path::Path;

use assert_cmd::Command;
use evalsig_store::objects::ObjectStore;
use serde_json::Value;

const CONFIG: &str = "[project]\nname = \"purge\"\n";
const SECRET: &str = "Jane Roe";

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--project"])
        .arg(dir);
    c
}

fn json(c: &mut Command) -> Value {
    serde_json::from_slice(
        &c.args(["--format", "json"])
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .unwrap()
}

/// Six items in their own strata; item `t-0` names the body and carries the text in
/// every free-text field.
fn records(body: &str) -> String {
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
        format!(
            r#"{{"record":"content","ref":"{body}","media_type":"text/plain","stored":"full"}}"#
        ),
    ];
    for i in 0..6 {
        let (attrs, strata, reason, detail) = if i == 0 {
            (
                format!(r#"{{"caller":"{SECRET}"}}"#),
                format!("{SECRET} household"),
                format!("{SECRET} confirmed"),
                format!("{SECRET} hung up"),
            )
        } else {
            (
                r#"{"caller":"n/a"}"#.into(),
                format!("group-{i}"),
                "ok".into(),
                "none".into(),
            )
        };
        let input = if i == 0 {
            format!(r#","input_ref":"{body}""#)
        } else {
            String::new()
        };
        lines.push(format!(
            r#"{{"record":"item","item_id":"t-{i}"{input},"attrs":{attrs},"strata":{{"channel":"{strata}"}}}}"#
        ));
        lines.push(format!(
            r#"{{"record":"label","item_id":"t-{i}","rater_id":"rater:a","source":"human","value":{},"version":1,"reason":"{reason}"}}"#,
            i % 2 == 0
        ));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"t-{i}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{}}}"#,
            i % 3 == 0
        ));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"t-{i}","run_id":"r","gen_replicate":1,"output_type":"binary","missing":{{"reason":"error","detail":"{detail}"}}}}"#
        ));
    }
    lines.join("\n") + "\n"
}

/// Every file under `.evalsig/` that contains the text, objects decompressed.
fn holding(root: &Path, text: &str) -> Vec<String> {
    let store = ObjectStore::new(root.join("objects"));
    let mut found: Vec<String> = store
        .list()
        .unwrap()
        .into_iter()
        .filter(|h| String::from_utf8_lossy(&store.get(h).unwrap()).contains(text))
        .map(|h| h.to_string())
        .collect();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p != root.join("objects") {
                    dirs.push(p);
                }
            } else if String::from_utf8_lossy(&std::fs::read(&p).unwrap()).contains(text) {
                found.push(p.display().to_string());
            }
        }
    }
    found
}

#[test]
fn purge_removes_the_text_and_keeps_the_ledger_valid() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".evalsig");
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(root.join("config.toml"), CONFIG).unwrap();
    let (body, _) = ObjectStore::new(root.join("objects"))
        .put(format!("{SECRET} called about the invoice").as_bytes())
        .unwrap();
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, records(&body.to_string())).unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    std::fs::remove_file(&file).unwrap();
    let before = json(evalsig(dir.path()).args(["report", "r", "--by", "channel"]));
    assert!(!holding(&root, SECRET).is_empty());

    evalsig(dir.path())
        .args(["purge", &body.to_string(), "--reason", "deletion request"])
        .assert()
        .code(64);
    let v = json(evalsig(dir.path()).args([
        "purge",
        &body.to_string(),
        "--reason",
        "deletion request",
        "--yes",
    ]));
    assert_eq!(v["results"]["items"], serde_json::json!(["t-0"]));
    assert_eq!(v["results"]["records_changed"], 4);
    assert_eq!(v["results"]["deleted_reports"].as_array().unwrap().len(), 1);

    assert_eq!(holding(&root, SECRET), Vec::<String>::new());
    evalsig(dir.path())
        .args(["ledger", "verify"])
        .assert()
        .success();
    let ledger = std::fs::read_to_string(root.join("ledger.jsonl")).unwrap();
    let last: Value = serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "purge");
    assert_eq!(last["summary"]["reason"], "deletion request");

    // The data still load, with the purged item in a stratum of its own.
    let after = json(evalsig(dir.path()).args(["report", "r"]));
    assert_eq!(after["results"]["n_items"], before["results"]["n_items"]);
    evalsig(dir.path())
        .args(["purge", &body.to_string(), "--reason", "again", "--yes"])
        .assert()
        .success();

    // Importing the purged records again is refused and leaves no text behind.
    std::fs::write(&file, records(&body.to_string())).unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .code(3);
    std::fs::remove_file(&file).unwrap();
    assert_eq!(holding(&root, SECRET), Vec::<String>::new());
}

#[test]
fn redact_mode_hides_matches_in_imported_fields() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".evalsig");
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(
        root.join("config.toml"),
        format!(
            "{CONFIG}[store]\ncontent = \"redact\"\nredact_patterns = [\"(?i)jane\\\\s+roe\"]\n"
        ),
    )
    .unwrap();
    let file = dir.path().join("records.jsonl");
    let body = "blake3:2222222222222222222222222222222222222222222222222222222222222222";
    std::fs::write(&file, records(body)).unwrap();
    let v = json(evalsig(dir.path()).arg("import").arg(&file));
    assert_eq!(v["results"]["redacted"], 3);
    assert_eq!(holding(&root, SECRET), Vec::<String>::new());
    assert!(!holding(&root, "[redacted] household").is_empty());

    std::fs::write(
        root.join("config.toml"),
        format!("{CONFIG}[store]\ncontent = \"redact\"\nredact_patterns = [\"(\"]\n"),
    )
    .unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .code(3);
}
