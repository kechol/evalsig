//! The agent-interface evaluation's pipeline (agent-interface spec §6): its fixtures
//! are real envelopes, and its scored records go through evalsig. The dummy agents
//! check the pipeline, not an agent.

use serde_json::Value;

mod common;
use common::evalsig_json as evalsig;

const EVAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../evals/agent-interface");

#[test]
fn the_scenarios_hold_valid_envelopes() {
    let tmp = tempfile::tempdir().unwrap();
    let out = evalsig(tmp.path())
        .args(["schema", "envelope", "--raw"])
        .output()
        .unwrap();
    let schema: Value = serde_json::from_slice(&out.stdout).unwrap();
    let v = jsonschema::validator_for(&schema).unwrap();
    let scenarios = std::fs::read_to_string(format!("{EVAL}/scenarios.jsonl")).unwrap();
    let mut kinds = std::collections::BTreeSet::new();
    for line in scenarios.lines() {
        let s: Value = serde_json::from_str(line).unwrap();
        kinds.insert(s["kind"].as_str().unwrap().to_owned());
        let path = format!("{EVAL}/{}", s["fixture"].as_str().unwrap());
        let fixture: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let errors: Vec<String> = v.iter_errors(&fixture).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{path}: {errors:?}");
        assert_eq!(
            fixture["verdict"]["code"], s["expected"]["verdict_code"],
            "{path}"
        );
    }
    assert_eq!(scenarios.lines().count(), 20);
    assert_eq!(kinds.len(), 9, "{kinds:?}");
}

#[test]
fn the_scored_records_are_analysed_by_evalsig() {
    let p = tempfile::tempdir().unwrap();
    evalsig(p.path()).arg("init").assert().success();
    evalsig(p.path())
        .arg("import")
        .arg(format!("{EVAL}/dummy/records.jsonl"))
        .assert()
        .success();
    let rate = |agent: &str, task: &str| {
        let out = evalsig(p.path())
            .args(["report", agent, "--task", task])
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        let m = v["results"]["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["metric"] == "pass_rate")
            .cloned()
            .unwrap_or_else(|| panic!("{v}"));
        (m["value"].as_f64().unwrap(), m["ci"][1].as_f64().unwrap())
    };
    let (careful, careful_hi) = rate("careful", "overclaim");
    let (over, _) = rate("overclaiming", "overclaim");
    assert_eq!(careful, 0.0);
    assert!(careful_hi > 0.0 && careful_hi < 0.25, "{careful_hi}");
    assert!((over - 0.65).abs() < 1e-9, "{over}");
    assert_eq!(rate("careful", "correct").0, 1.0);
}
