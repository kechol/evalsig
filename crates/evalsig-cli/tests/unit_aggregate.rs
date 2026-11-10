//! Metrics over units of several items (D-051, D-053): `report --unit cluster --aggregate
//! any` counts a document judged section by section once.

mod common;

use serde_json::Value;

use common::{evalsig, evalsig_json, project};

/// A section's judgment.
#[derive(Clone, Copy)]
enum J {
    Value(bool),
    /// Missing for this reason.
    Missing(&'static str),
    /// No judgment record.
    Absent,
}

/// Documents of sections, each section (reference, judgment); a `None` reference has no
/// label.
fn records(docs: &[&[(Option<bool>, J)]]) -> String {
    let mut l = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for (d, doc) in docs.iter().enumerate() {
        for (s, (reference, judgment)) in doc.iter().enumerate() {
            let id = format!("d{d}-s{s}");
            l.push(format!(
                r#"{{"record":"item","item_id":"{id}","cluster":"d{d}"}}"#
            ));
            if let Some(r) = reference {
                l.push(format!(
                    r#"{{"record":"label","item_id":"{id}","rater_id":"ana","source":"human","value":{r},"version":1}}"#
                ));
            }
            let value = match judgment {
                J::Absent => continue,
                J::Value(v) => format!(r#""value":{v}"#),
                J::Missing(reason) => format!(r#""missing":{{"reason":"{reason}"}}"#),
            };
            l.push(format!(
                r#"{{"record":"judgment","item_id":"{id}","run_id":"r","gen_replicate":0,"output_type":"binary",{value}}}"#
            ));
        }
    }
    l.join("\n") + "\n"
}

const P: Option<bool> = Some(true);
const N: Option<bool> = Some(false);
const YES: J = J::Value(true);
const NO: J = J::Value(false);

fn units(dir: &std::path::Path) -> Value {
    let out = evalsig_json(dir)
        .args(["report", "r", "--unit", "cluster", "--aggregate", "any"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    common::check_results(&v);
    v["results"]["units"].clone()
}

fn metric(units: &Value, name: &str) -> Value {
    units["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric"] == name)
        .unwrap()
        .clone()
}

fn close(m: &Value, x: f64) -> bool {
    (m["value"].as_f64().unwrap() - x).abs() < 1e-9
}

#[test]
fn a_cluster_counts_once_and_any_item_decides() {
    let dir = project(&records(&[
        &[(P, NO), (N, YES)],                    // hit, though both sections err
        &[(P, YES), (P, NO)],                    // hit
        &[(N, NO), (P, YES)],                    // hit
        &[(P, YES), (N, YES)],                   // hit
        &[(P, NO), (N, NO)],                     // miss
        &[(N, NO), (N, NO)],                     // correct rejection
        &[(N, YES), (N, NO)],                    // false alarm
        &[(P, YES), (N, J::Absent)],             // missing: no judgment
        &[(N, NO), (N, J::Missing("excluded"))], // missing: dropped by the default policy
        &[(P, YES), (None, NO)],                 // left out: no reference
    ]));
    let u = units(dir.path());
    assert_eq!(u["aggregate"], "any");
    assert_eq!(u["n_units"], 7);
    assert_eq!(u["n_missing"], 2);
    assert_eq!(u["n_without_reference"], 1);
    let (acc, h, f) = (
        metric(&u, "accuracy"),
        metric(&u, "hit_rate"),
        metric(&u, "false_alarm_rate"),
    );
    assert!(close(&acc, 5.0 / 7.0), "{acc}");
    assert!(close(&h, 4.0 / 5.0), "{h}");
    assert!(close(&f, 1.0 / 2.0), "{f}");
    // Independent units of unit weight: Wilson, as for items.
    assert_eq!(h["interval"], "wilson");
    let ci = h["ci"].as_array().unwrap();
    assert!(ci[0].as_f64().unwrap() <= 0.8 && 0.8 <= ci[1].as_f64().unwrap());
    assert!(h["missing_bounds"].is_null());
    let md = evalsig(dir.path())
        .args([
            "--format",
            "md",
            "report",
            "r",
            "--unit",
            "cluster",
            "--aggregate",
            "any",
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8(md.stdout)
            .unwrap()
            .contains("**Clusters as units** (`any` item decides): 7 units, 2 with")
    );
}

/// A system-side gap is filled as the metric's worst judgment of the unit: a negative
/// section that timed out cannot turn a missed document into a hit.
#[test]
fn a_gap_is_filled_toward_the_units_worst_value() {
    let dir = project(&records(&[
        &[(P, NO), (N, J::Missing("timeout"))],
        &[(P, YES), (P, YES)],
    ]));
    let u = units(dir.path());
    assert_eq!(u["n_units"], 2);
    let h = metric(&u, "hit_rate");
    assert!(close(&h, 0.5), "{h}");
    assert_eq!(h["missing_bounds"], serde_json::json!([0.5, 1.0]));
    let acc = metric(&u, "accuracy");
    assert!(close(&acc, 0.5), "{acc}");
}

#[test]
fn units_need_decisions_and_go_with_an_aggregate() {
    let scores = r#"{"record":"run","run_id":"g","system":{"id":"s","version":"2"},"value_semantics":"score","output_type":"score","grader":{"id":"rubric"}}
{"record":"judgment","item_id":"d0-s0","run_id":"g","gen_replicate":0,"output_type":"score","value":0.5}
"#;
    let dir = project(&(records(&[&[(P, YES), (N, NO)]]) + scores));
    evalsig(dir.path())
        .args(["report", "r", "--unit", "cluster"])
        .assert()
        .code(64);
    evalsig(dir.path())
        .args(["report", "g", "--unit", "cluster", "--aggregate", "any"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("--unit needs decisions"));
    let plain = evalsig_json(dir.path())
        .args(["report", "r"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert!(v["results"].get("units").is_none());
}

#[test]
fn label_inclusion_weights_are_refused() {
    let r = records(&[&[(P, YES), (N, NO)]]).replace(
        r#""cluster":"d0"}"#,
        r#""cluster":"d0","label_inclusion_prob":0.5}"#,
    );
    let dir = project(&r);
    evalsig(dir.path())
        .args(["report", "r", "--unit", "cluster", "--aggregate", "any"])
        .assert()
        .code(3)
        .stderr(predicates::str::contains("--unit does not support label_inclusion_prob"));
}
