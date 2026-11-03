//! `report --k-curve` against hand-computed values.
//!
//! Item a is positive with replicates (true, true, false): c = 2 of K = 3. Item b is
//! negative with (false, true, false): c = 1. With C(n, k) the binomial coefficient:
//! - any@k = 1 − C(K−c, k)/C(K, k): a 2/3, 1, 1; b 1/3, 2/3, 1
//! - all@k = C(c, k)/C(K, k): a 2/3, 1/3, 0; b 1/3, 0, 0
//! - majority@k (a tie counts half): a 2/3, 2/3, 1; b 1/3, 1/3, 0
//!
//! The hit rate is a's value and the false-alarm rate b's.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "json", "--project"])
        .arg(dir);
    c
}

const RECORDS: &str = r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}
{"record":"item","item_id":"a"}
{"record":"item","item_id":"b"}
{"record":"label","item_id":"a","rater_id":"h","source":"human","value":true,"version":1}
{"record":"label","item_id":"b","rater_id":"h","source":"human","value":false,"version":1}
{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"binary","value":true,"cost":{"usd":0.01}}
{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":1,"output_type":"binary","value":true,"cost":{"usd":0.01}}
{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":2,"output_type":"binary","value":false,"cost":{"usd":0.01}}
{"record":"judgment","item_id":"b","run_id":"r","gen_replicate":0,"output_type":"binary","value":false,"cost":{"usd":0.01}}
{"record":"judgment","item_id":"b","run_id":"r","gen_replicate":1,"output_type":"binary","value":true,"cost":{"usd":0.01}}
{"record":"judgment","item_id":"b","run_id":"r","gen_replicate":2,"output_type":"binary","value":false,"cost":{"usd":0.01}}
"#;

#[test]
fn the_k_curve_matches_hand_computed_values() {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let file = dir.path().join("r.jsonl");
    std::fs::write(&file, RECORDS).unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    let out = evalsig(dir.path())
        .args(["report", "r", "--k-curve"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let curve = &v["results"]["k_curve"];
    assert_eq!(curve["k_max"], 3);
    let points = curve["points"].as_array().unwrap();
    assert_eq!(points.len(), 9);
    let metric = |rule: &str, k: u64, name: &str| -> f64 {
        let p = points
            .iter()
            .find(|p| p["rule"] == rule && p["k"] == k)
            .unwrap();
        p["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["metric"] == name)
            .unwrap()["value"]
            .as_f64()
            .unwrap()
    };
    let third = 1.0 / 3.0;
    let expected = [
        (
            "any",
            [(2.0 * third, third), (1.0, 2.0 * third), (1.0, 1.0)],
        ),
        ("all", [(2.0 * third, third), (third, 0.0), (0.0, 0.0)]),
        (
            "majority",
            [(2.0 * third, third), (2.0 * third, third), (1.0, 0.0)],
        ),
    ];
    for (rule, by_k) in expected {
        for (i, (h, f)) in by_k.into_iter().enumerate() {
            let k = i as u64 + 1;
            assert!(
                (metric(rule, k, "hit_rate") - h).abs() < 1e-9,
                "{rule}@{k} H"
            );
            assert!(
                (metric(rule, k, "false_alarm_rate") - f).abs() < 1e-9,
                "{rule}@{k} F"
            );
        }
    }
    let cost = points
        .iter()
        .find(|p| p["rule"] == "any" && p["k"] == 2)
        .unwrap()["cost_per_item"]["usd"]
        .as_f64()
        .unwrap();
    assert!((cost - 0.02).abs() < 1e-9);
}
