//! Feature direction and lineage (D-049): `report --features` says whether each feature
//! points the way its run declares.

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
