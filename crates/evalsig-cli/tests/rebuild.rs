use assert_cmd::Command;
use serde_json::Value;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);

fn evalsig(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

#[test]
fn deleting_the_cache_and_rebuilding_gives_identical_query_results() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    evalsig(dir.path())
        .args(["import", SYNTHETIC])
        .assert()
        .success();
    let out = evalsig(dir.path())
        .arg("rebuild")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["results"]["rows"]["judgments"], 600);
    let db = dir.path().join(".evalsig/cache/evalsig.db");
    let first = evalsig_store::cache::dump(&evalsig_store::cache::open(&db).unwrap()).unwrap();
    std::fs::remove_dir_all(dir.path().join(".evalsig/cache")).unwrap();
    evalsig(dir.path()).arg("rebuild").assert().success();
    let second = evalsig_store::cache::dump(&evalsig_store::cache::open(&db).unwrap()).unwrap();
    assert_eq!(first, second);
    assert!(first.lines().count() > 1000);
}
