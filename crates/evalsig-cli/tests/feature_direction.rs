//! Feature direction and lineage (D-049): `report --features` says whether each feature
//! points the way its run declares.

mod common;

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("AI_AGENT")
        .arg("--project")
        .arg(dir);
    c
}

/// 100 positives and 100 negatives. `pos` fires on 60 positives and 5 negatives,
/// `neg` the other way round, `flat` on every fifth item, `rare` on two items.
fn records(features: &str) -> String {
    let mut l = vec![format!(
        r#"{{"record":"run","run_id":"r","system":{{"id":"s","version":"1"}},"output_type":"binary","features":{features}}}"#
    )];
    for i in 0..200 {
        let positive = i < 100;
        let k = i % 100;
        let fired = |on: bool, id: &str| format!(r#"{{"feature_id":"{id}","fired":{on}}}"#);
        let fs = [
            fired(if positive { k < 60 } else { k < 5 }, "pos"),
            fired(if positive { k < 5 } else { k < 60 }, "neg"),
            fired(i % 5 == 0, "flat"),
            fired(i == 0 || i == 150, "rare"),
            fired(if positive { k < 60 } else { k < 5 }, "undeclared"),
        ]
        .join(",");
        l.push(format!(r#"{{"record":"item","item_id":"i{i:03}"}}"#));
        l.push(format!(
            r#"{{"record":"label","item_id":"i{i:03}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
        ));
        l.push(format!(
            r#"{{"record":"judgment","item_id":"i{i:03}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{positive},"features":[{fs}]}}"#
        ));
    }
    l.join("\n") + "\n"
}

#[test]
fn each_feature_is_read_against_the_side_it_should_point_to() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let file = dir.path().join("r.jsonl");
    std::fs::write(
        &file,
        records(
            r#"{"pos":{"expected":"positive"},"neg":{"expected":"positive"},"flat":{"expected":"negative"},"rare":{"expected":"negative"}}"#,
        ),
    )
    .unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    let out = evalsig(dir.path())
        .args(["--format", "json", "report", "r", "--features"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    common::check_results(&v);
    let direction = |id: &str| {
        v["results"]["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["feature_id"] == id)
            .map(|f| f["direction"].clone())
            .unwrap()
    };
    assert_eq!(direction("pos"), "as_expected");
    assert_eq!(direction("neg"), "reversed");
    assert_eq!(direction("flat"), "uninformative");
    assert_eq!(direction("rare"), "insufficient");
    assert!(direction("undeclared").is_null());
    let out = evalsig(dir.path())
        .args(["--format", "md", "report", "r", "--features"])
        .output()
        .unwrap();
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(
        md.contains("| Direction |") && md.contains("`reversed`"),
        "{md}"
    );
}

#[test]
fn a_split_rule_is_followed_through_its_lineage() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let mut l = vec![
        r#"{"record":"run","run_id":"before","system":{"id":"s","version":"1"},"output_type":"binary"}"#.to_owned(),
        r#"{"record":"run","run_id":"after","system":{"id":"s","version":"2"},"output_type":"binary","features":{"R10-1":{"lineage":"R10"},"R10-2":{"lineage":"R10"}}}"#.to_owned(),
    ];
    for i in 0..200 {
        let positive = i < 100;
        let k = i % 100;
        // R10 fires on 60 positives and 5 negatives; R10-1 and R10-2 split those items.
        let r10 = if positive { k < 60 } else { k < 5 };
        let first = if positive { k < 30 } else { k < 3 };
        let f = |id: &str, on: bool| format!(r#"{{"feature_id":"{id}","fired":{on}}}"#);
        l.push(format!(r#"{{"record":"item","item_id":"i{i:03}"}}"#));
        l.push(format!(
            r#"{{"record":"label","item_id":"i{i:03}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
        ));
        l.push(format!(
            r#"{{"record":"judgment","item_id":"i{i:03}","run_id":"before","gen_replicate":0,"output_type":"binary","value":{positive},"features":[{}]}}"#,
            f("R10", r10)
        ));
        l.push(format!(
            r#"{{"record":"judgment","item_id":"i{i:03}","run_id":"after","gen_replicate":0,"output_type":"binary","value":{positive},"features":[{},{}]}}"#,
            f("R10-1", r10 && first),
            f("R10-2", r10 && !first)
        ));
    }
    let file = dir.path().join("r.jsonl");
    std::fs::write(&file, l.join("\n") + "\n").unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    let out = evalsig(dir.path())
        .args([
            "--format",
            "json",
            "compare",
            "before",
            "after",
            "--features",
            "--scratch",
        ])
        .output()
        .unwrap();
    assert!(out.status.code().is_some_and(|c| c < 64), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    common::check_results(&v);
    let rows = v["results"]["features"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["lineage"], "R10");
    assert_eq!(
        rows[0]["baseline"]["lr"], rows[0]["candidate"]["lr"],
        "{rows:?}"
    );
    assert_eq!(rows[0]["baseline"]["lr"], 12.0);
    // Markdown lists the lineage in both columns.
    let out = evalsig(dir.path())
        .args(["--format", "md", "compare", "before", "after"])
        .args(["--features", "--scratch"])
        .output()
        .unwrap();
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(md.contains("| R10 | 12.0000 |"), "{md}");
}
