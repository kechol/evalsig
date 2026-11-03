//! `evalsig export` (D-025): Parquet that DuckDB reads with matching row counts, CSV
//! and canonical JSONL.

use std::path::Path;
use std::process::Command as Process;

use assert_cmd::Command;
use parquet::file::reader::{FileReader, SerializedFileReader};
use serde_json::Value;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .arg("--project")
        .arg(dir);
    c
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    evalsig(dir.path())
        .args(["import", SYNTHETIC])
        .assert()
        .success();
    dir
}

fn export(dir: &Path, to: &str, extra: &[&str]) -> Value {
    let out = evalsig(dir)
        .args(["--format", "json", "export", "--to", to, "--out"])
        .arg(dir.join(to))
        .args(extra)
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

/// Records of each type in the synthetic example.
fn source_counts() -> std::collections::BTreeMap<String, usize> {
    let mut m = std::collections::BTreeMap::new();
    for line in std::fs::read_to_string(SYNTHETIC).unwrap().lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        let t = match v["record"].as_str().unwrap() {
            "run" => "runs",
            "item" => "items",
            "judgment" => "judgments",
            "label" => "labels",
            "preference" => "preferences",
            other => panic!("{other}"),
        };
        *m.entry(t.to_owned()).or_insert(0) += 1;
    }
    m
}

#[test]
fn parquet_row_counts_match_and_duckdb_reads_them() {
    let dir = project();
    let v = export(dir.path(), "parquet", &["--items", "triage-v1"]);
    let counts = source_counts();
    for t in v["results"]["tables"].as_array().unwrap() {
        let name = t["table"].as_str().unwrap();
        let file = std::fs::File::open(t["file"].as_str().unwrap()).unwrap();
        let rows = SerializedFileReader::new(file)
            .unwrap()
            .metadata()
            .file_metadata()
            .num_rows();
        assert_eq!(rows as u64, t["rows"].as_u64().unwrap(), "{name}");
        let expected = if name == "items_triage-v1" {
            300
        } else {
            counts[name]
        };
        assert_eq!(rows as usize, expected, "{name}");
    }
    // DuckDB is the reader the spec names; checked when it is installed.
    let judgments = dir.path().join("parquet/judgments.parquet");
    if let Ok(out) = Process::new("duckdb")
        // A plain CSV answer, whatever the caller's environment asks of DuckDB.
        .env_remove("AI_AGENT")
        .args(["-noheader", "-csv", "-c"])
        .arg(format!("select count(*) from '{}'", judgments.display()))
        .output()
        && out.status.success()
    {
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            counts["judgments"].to_string()
        );
    } else {
        eprintln!("duckdb not installed; Parquet checked with the parquet reader only");
    }
}

#[test]
fn csv_and_jsonl_hold_every_record() {
    let dir = project();
    let counts = source_counts();
    export(dir.path(), "csv", &[]);
    let csv = std::fs::read_to_string(dir.path().join("csv/items.csv")).unwrap();
    let mut lines = csv.split("\r\n").filter(|l| !l.is_empty());
    let header = lines.next().unwrap();
    assert!(header.split(',').any(|c| c == "item_id") && header.split(',').any(|c| c == "strata"));
    let first = lines.next().unwrap();
    // Strata are canonical JSON text, quoted because they hold quotes.
    assert!(first.contains(r#""{""channel"":"#), "{first}");
    assert_eq!(
        csv.split("\r\n").filter(|l| !l.is_empty()).count() - 1,
        counts["items"]
    );

    export(dir.path(), "jsonl", &[]);
    let jsonl = std::fs::read_to_string(dir.path().join("jsonl/judgments.jsonl")).unwrap();
    assert_eq!(jsonl.lines().count(), counts["judgments"]);
    let first: Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    assert_eq!(first["record"], "judgment");
}
