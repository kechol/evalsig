use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

const RECORDS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl");
const LABELS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/human-labels.csv"
);

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

fn json(out: &[u8]) -> Value {
    serde_json::from_slice(out).unwrap()
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    dir
}

#[test]
fn records_and_a_csv_label_file_import_into_one_snapshot() {
    let dir = project();
    let out = evalsig(dir.path())
        .args(["import", RECORDS, LABELS, "--as", "label"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(&out);
    assert_eq!(v["schema"], "evalsig.import/1");
    assert_eq!(v["results"]["records"]["labels"], 8 + 3);
    assert_eq!(v["results"]["records"]["judgments"], 7);
    assert_eq!(v["results"]["sources"].as_array().unwrap().len(), 2);
    let snapshot = v["results"]["snapshot"].as_str().unwrap().to_owned();
    assert!(snapshot.starts_with("blake3:"));
    assert_eq!(v["provenance"]["snapshots"][0], snapshot);

    let ledger = std::fs::read_to_string(dir.path().join(".evalsig/ledger.jsonl")).unwrap();
    let entry: Value = serde_json::from_str(ledger.lines().next().unwrap()).unwrap();
    assert_eq!(entry["kind"], "import");
    assert_eq!(entry["actor"], "user:test");
    assert_eq!(entry["outputs"][0], snapshot);
    assert_eq!(entry["summary"]["labels"], 11);
    assert!(
        evalsig_store::ledger::verify(&ledger, Some(evalsig_store::ledger::LedgerSide::Agent))
            .is_empty()
    );

    // The unmappable label value became missing, not a number.
    let store = evalsig_store::objects::ObjectStore::new(dir.path().join(".evalsig/objects"));
    let records = evalsig_store::snapshot::read(&store, &snapshot.parse().unwrap()).unwrap();
    let missing = records
        .iter()
        .filter(|r| matches!(r, evalsig_core::records::Record::Label(l) if l.missing.is_some()))
        .count();
    assert_eq!(missing, 1);

    // Importing again gives the same snapshot.
    let again = json(
        &evalsig(dir.path())
            .args(["import", RECORDS, LABELS, "--as", "label"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(again["results"]["snapshot"], snapshot);
    assert_eq!(again["provenance"]["ledger_seq"], 1);
}

#[test]
fn overrides_and_converter_are_recorded() {
    let dir = project();
    let converter = dir.path().join("convert.py");
    std::fs::write(&converter, "print('convert')\n").unwrap();
    let v = json(
        &evalsig(dir.path())
            .args([
                "import",
                RECORDS,
                "--system-version",
                "v9",
                "--grader",
                "judge@2",
                "--converter",
            ])
            .arg(&converter)
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    let store = evalsig_store::objects::ObjectStore::new(dir.path().join(".evalsig/objects"));
    let prov = evalsig_store::snapshot::read_provenance(
        &store,
        &v["results"]["provenance"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        prov.mapping,
        Some(evalsig_store::hash::ContentHash::blake3(
            b"print('convert')\n"
        ))
    );
    let records = evalsig_store::snapshot::read(&store, &prov.snapshot).unwrap();
    for r in records {
        if let evalsig_core::records::Record::Run(run) = r {
            assert_eq!(run.system.version, "v9");
            assert_eq!(run.grader.unwrap().id.as_deref(), Some("judge"));
        }
    }
}

#[test]
fn invalid_input_blocks_the_import() {
    let dir = project();
    let bad = dir.path().join("bad.jsonl");
    std::fs::write(&bad, "{\"record\":\"item\",\"item_id\":\"a\"}\n{\"record\":\"item\",\"item_id\":\"a\",\"weight\":2}\n").unwrap();
    let v = json(
        &evalsig(dir.path())
            .args(["import"])
            .arg(&bad)
            .assert()
            .code(5)
            .get_output()
            .stdout,
    );
    assert_eq!(v["verdict"]["code"], "blocked");
    assert_eq!(v["results"]["issues"][0]["code"], "record.duplicate_key");
    assert!(!dir.path().join(".evalsig/ledger.jsonl").exists());
    evalsig(dir.path())
        .args(["import", "data.txt"])
        .assert()
        .code(64);
}

#[test]
fn import_needs_an_initialized_project() {
    let dir = tempfile::tempdir().unwrap();
    let out = evalsig(dir.path())
        .args(["import", RECORDS])
        .assert()
        .code(64)
        .get_output()
        .stderr
        .clone();
    assert_eq!(json(&out)["error"]["code"], "not_initialized");
}

#[test]
fn dry_run_writes_nothing() {
    let dir = project();
    let v = json(
        &evalsig(dir.path())
            .args(["import", RECORDS, "--dry-run"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(v["results"]["dry_run"], true);
    assert!(!dir.path().join(".evalsig/ledger.jsonl").exists());
}

#[test]
fn csv_rows_get_the_schema_checks() {
    let dir = project();
    let csv = dir.path().join("labels.csv");
    std::fs::write(
        &csv,
        "item_id,rater_id,value,labeled_at\na,u,true,yesterday\n",
    )
    .unwrap();
    let v = json(
        &evalsig(dir.path())
            .args(["import", "--as", "label"])
            .arg(&csv)
            .assert()
            .code(5)
            .get_output()
            .stdout,
    );
    assert_eq!(v["results"]["issues"][0]["line"], 2);
    assert_eq!(v["results"]["issues"][0]["code"], "schema.format");
}
