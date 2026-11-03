//! Importing promptfoo results with `--adapter promptfoo` and `[[import]]` blocks.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/harness/promptfoo/out.json")
}

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

fn json(c: &mut Command) -> Value {
    serde_json::from_slice(&c.assert().success().get_output().stdout).unwrap()
}

#[test]
fn results_import_with_replicates_and_report() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let v = json(
        evalsig(dir.path())
            .args(["import", "--adapter", "promptfoo"])
            .arg(fixture()),
    );
    assert_eq!(
        v["results"]["records"],
        serde_json::json!({"runs": 1, "items": 2, "judgments": 4, "contents": 4})
    );
    let run = json(evalsig(dir.path()).arg("runs"))["results"]["runs"][0]["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(run.starts_with("eval-"), "{run}");
    let report = json(evalsig(dir.path()).args(["report", &run]));
    assert_eq!(report["results"]["n_judgments"], 4);
    assert_eq!(report["results"]["n_items"], 2);
    // Each item has two generation replicates, so reproducibility is reported.
    let rep = &report["results"]["reproducibility"];
    assert!(rep.as_object().is_some_and(|m| !m.is_empty()), "{rep}");
}

#[test]
fn a_promptfoo_import_block_applies() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::copy(fixture(), dir.path().join("out.json")).unwrap();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[[import]]\nname = \"pf\"\nadapter = \"promptfoo\"\npath = \"out.json\"\n",
    )
    .unwrap();
    let v = json(evalsig(dir.path()).args(["import", "--from", "pf"]));
    assert_eq!(v["results"]["records"]["judgments"], 4);
    assert!(
        v["results"]["sources"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("out.json")
    );
}
