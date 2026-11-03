//! `import --mapping` and `import --from` (D-011).

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn example(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(rel)
}

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    dir
}

fn json(c: &mut Command, code: i32) -> Value {
    let out = c.assert().code(code).get_output().clone();
    let bytes = if code == 0 { out.stdout } else { out.stderr };
    serde_json::from_slice(&bytes).unwrap()
}

fn import_example_mapping(dir: &Path, mapping: &Path, log: &Path) -> Command {
    let mut c = evalsig(dir);
    c.arg("import")
        .arg("--mapping")
        .arg(mapping)
        .args(["--arg", "run-id=triage-v1", "--arg"])
        .arg(format!(
            "labels={}",
            example("custom-harness/labels.sample.json").display()
        ))
        .arg(log);
    c
}

#[test]
fn the_example_mapping_equals_the_converter_script() {
    let mapped = project();
    let v = json(
        &mut import_example_mapping(
            mapped.path(),
            &example("mapping.toml"),
            &example("custom-harness/run-log.sample.jsonl"),
        ),
        0,
    );
    let converted = project();
    let w = json(
        evalsig(converted.path())
            .arg("import")
            .arg(example("custom-harness/records.out.jsonl")),
        0,
    );
    assert_eq!(v["results"]["snapshot"], w["results"]["snapshot"]);
    assert_eq!(v["results"]["sources"].as_array().unwrap().len(), 2);
    let ledger = std::fs::read_to_string(mapped.path().join(".evalsig/ledger.jsonl")).unwrap();
    let line: Value = serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
    let mapping = std::fs::read(example("mapping.toml")).unwrap();
    let hash = format!("blake3:{}", blake3_hex(&mapping));
    assert!(
        line["inputs"]
            .as_array()
            .unwrap()
            .contains(&Value::String(hash))
    );
    assert_eq!(line["summary"]["adapter"], "mapping:triage-run");
}

fn blake3_hex(bytes: &[u8]) -> String {
    evalsig_store::hash::ContentHash::blake3(bytes).hex()
}

#[test]
fn a_mapping_with_decide_left_is_refused() {
    let dir = project();
    let text = std::fs::read_to_string(example("mapping.toml"))
        .unwrap()
        .replace(
            r#"value_semantics = "decision""#,
            r#"value_semantics = "DECIDE""#,
        );
    let mapping = dir.path().join("mapping.toml");
    std::fs::write(&mapping, text).unwrap();
    let v = json(
        &mut import_example_mapping(
            dir.path(),
            &mapping,
            &example("custom-harness/run-log.sample.jsonl"),
        ),
        5,
    );
    assert_eq!(v["error"]["code"], "undecided_mapping");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("run.value_semantics")
    );
}

#[test]
fn unknown_versions_stop_and_unmapped_values_become_missing() {
    let dir = project();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[project]\npositive = { equals = true }\ndecision = { ordinal_at_least = 2 }\n",
    )
    .unwrap();
    let log = dir.path().join("run.jsonl");
    let base = std::fs::read_to_string(example("custom-harness/run-log.sample.jsonl")).unwrap();
    std::fs::write(&log, base.replace("triage-run/1", "triage-run/2")).unwrap();
    let v = json(
        &mut import_example_mapping(dir.path(), &example("mapping.toml"), &log),
        3,
    );
    assert_eq!(v["error"]["code"], "unexpected_source");

    std::fs::write(
        &log,
        base.replacen(r#""level":"urgent""#, r#""level":"extreme""#, 1),
    )
    .unwrap();
    json(
        &mut import_example_mapping(dir.path(), &example("mapping.toml"), &log),
        0,
    );
    let rows = json(evalsig(dir.path()).args(["report", "triage-v1"]), 0);
    assert_eq!(rows["results"]["n_missing"], 2);
}

const CONFIG: &str = r#"[project]
positive = { equals = true }
decision = { ordinal_at_least = 2 }

[[import]]
name = "generic-jsonl"
adapter = "jsonl"
path = "data/judgments.jsonl"
map.item_id = "$.id"
map.gen_replicate = "$.trial"
map.value = "$.verdict"
map.output_type = "ordinal"
map.levels = 4
map.offset = 1
map.run_id = "$.run"
map.strata.lang = "$.meta.lang"
map.input_ref = "$.question"

[[import]]
name = "human-labels"
adapter = "csv"
as = "label"
path = "labels/*.csv"
map.item_id = "id"
map.rater_id = "annotator"
map.source = "human"
map.value = "label"
value_map = { yes = true, no = false }
map.version = 1
"#;

#[test]
fn import_blocks_of_the_configuration_apply() {
    let dir = project();
    let root = dir.path();
    std::fs::write(root.join(".evalsig/config.toml"), CONFIG).unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::create_dir_all(root.join("labels")).unwrap();
    let mut lines = String::new();
    for i in 0..6 {
        for trial in 0..2 {
            lines.push_str(&format!(
                "{{\"id\":\"q{i}\",\"trial\":{trial},\"verdict\":{},\"run\":\"r1\",\"meta\":{{\"lang\":\"en\"}},\"question\":\"Question {i}?\"}}\n",
                1 + (i + trial) % 4
            ));
        }
    }
    std::fs::write(root.join("data/judgments.jsonl"), lines).unwrap();
    let mut csv = "id,annotator,label\n".to_owned();
    for i in 0..6 {
        csv.push_str(&format!(
            "q{i},rater:a,{}\n",
            if i % 2 == 0 { "yes" } else { "maybe" }
        ));
    }
    std::fs::write(root.join("labels/human.csv"), csv).unwrap();

    let v = json(evalsig(root).args(["import", "--from", "generic-jsonl"]), 0);
    assert_eq!(v["results"]["records"]["judgments"], 12);
    assert_eq!(v["results"]["records"]["contents"], 6);
    let w = json(evalsig(root).args(["import", "--from", "human-labels"]), 0);
    assert_eq!(w["results"]["records"], serde_json::json!({"labels": 6}));

    // Bodies are stored by default (store.content = "full").
    let store = evalsig_store::objects::ObjectStore::new(root.join(".evalsig/objects"));
    let snapshot = v["results"]["snapshot"].as_str().unwrap().parse().unwrap();
    let records = evalsig_store::snapshot::read(&store, &snapshot).unwrap();
    let q0 = records
        .iter()
        .find_map(|r| match r {
            evalsig_core::records::Record::Item(i) if i.item_id == "q0" => Some(i),
            _ => None,
        })
        .unwrap();
    assert_eq!(q0.strata.as_ref().unwrap()["lang"], "en");
    let input = q0.input_ref.as_deref().unwrap().parse().unwrap();
    assert_eq!(store.get(&input).unwrap(), b"Question 0?");

    // "maybe" is not in value_map, so three labels are missing.
    let report = json(evalsig(root).args(["report", "r1"]), 0);
    assert_eq!(report["results"]["reference"]["items_with_reference"], 3);

    let e = json(evalsig(root).args(["import", "--from", "nope"]), 64);
    assert!(
        e["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("generic-jsonl")
    );
}

#[test]
fn the_example_mapping_conforms_to_the_published_schema() {
    let schema: Value = serde_json::from_str(
        &String::from_utf8(
            Command::cargo_bin("evalsig")
                .unwrap()
                .args(["schema", "mapping", "--raw"])
                .assert()
                .success()
                .get_output()
                .stdout
                .clone(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["$id"], "urn:evalsig:schema:mapping:1");
    let mapping: Value =
        toml::from_str(&std::fs::read_to_string(example("mapping.toml")).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&mapping)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}
