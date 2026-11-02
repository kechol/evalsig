use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

const RECORDS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl");

fn evalsig(dir: &Path, format: &str) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", format, "--project"])
        .arg(dir);
    c
}

fn imported() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path(), "json").arg("init").assert().success();
    evalsig(dir.path(), "json")
        .args(["import", RECORDS])
        .assert()
        .success();
    dir
}

fn stdout(c: &mut Command) -> String {
    String::from_utf8(c.assert().success().get_output().stdout.clone()).unwrap()
}

#[test]
fn runs_lists_every_run_with_counts() {
    let dir = imported();
    let v: Value = serde_json::from_str(&stdout(evalsig(dir.path(), "json").arg("runs"))).unwrap();
    assert_eq!(v["schema"], "evalsig.runs/1");
    assert_eq!(v["results"]["count"], 3);
    let base = v["results"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == "r-base")
        .unwrap();
    assert_eq!(base["judgments"], 4);
    assert_eq!(base["items"], 2);
    assert_eq!(base["missing"], 1);
    assert_eq!(v["provenance"]["snapshots"].as_array().unwrap().len(), 1);
}

#[test]
fn items_lists_every_item_with_counts() {
    let dir = imported();
    let v: Value = serde_json::from_str(&stdout(evalsig(dir.path(), "json").arg("items"))).unwrap();
    assert_eq!(v["results"]["count"], 3);
    let q1 = &v["results"]["items"][0];
    assert_eq!(q1["item_id"], "q-0001");
    assert_eq!(q1["cluster"], "doc-17");
    assert_eq!(q1["labels"], 6);
}

#[test]
fn listings_render_in_every_format() {
    let dir = imported();
    for cmd in ["runs", "items"] {
        let md = stdout(evalsig(dir.path(), "md").arg(cmd));
        assert!(md.contains(&format!("**{cmd}**")), "{md}");
        assert!(
            md.contains("| q-0001 |") || md.contains("| r-base |"),
            "{md}"
        );
        let text = stdout(evalsig(dir.path(), "text").arg(cmd));
        assert!(text.contains("q-0002") || text.contains("r-cand"), "{text}");
        let junit = stdout(evalsig(dir.path(), "junit").arg(cmd));
        assert!(junit.contains("failures=\"0\""), "{junit}");
    }
}

#[test]
fn a_later_import_of_the_same_key_wins() {
    let dir = imported();
    let newer = dir.path().join("newer.jsonl");
    std::fs::write(&newer, "{\"record\":\"run\",\"run_id\":\"r-base\",\"system\":{\"id\":\"classifier\",\"version\":\"prompt-v3b\"}}\n").unwrap();
    evalsig(dir.path(), "json")
        .arg("import")
        .arg(&newer)
        .assert()
        .success();
    let v: Value = serde_json::from_str(&stdout(evalsig(dir.path(), "json").arg("runs"))).unwrap();
    let base = v["results"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == "r-base")
        .unwrap();
    assert_eq!(base["version"], "prompt-v3b");
    assert_eq!(v["results"]["count"], 3);
}
