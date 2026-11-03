//! Score-type metrics and the @k estimands against the reference values.

use evalsig_core::score::{Scored, mean_score, pass_at_k, rate_at_least};
use serde_json::Value;

const REFERENCE: &str = include_str!("../../../tests/fixtures/reference-values.json");

fn close(a: f64, b: &Value, what: &str) {
    let b = b.as_f64().unwrap();
    assert!((a - b).abs() <= 1e-9, "{what}: {a} vs {b}");
}

#[test]
fn score_cases_match() {
    let doc: Value = serde_json::from_str(REFERENCE).unwrap();
    let cases = doc["cases"]["score"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for c in cases {
        let i = &c["input"];
        if let Some(scores) = i["scores"].as_array() {
            let items: Vec<Scored> = scores
                .iter()
                .zip(i["weights"].as_array().unwrap())
                .map(|(s, w)| Scored {
                    score: s.as_f64().unwrap(),
                    weight: w.as_f64().unwrap(),
                })
                .collect();
            let t = i["t"].as_f64().unwrap();
            close(
                mean_score(&items).unwrap(),
                &c["expected"]["mean_score"],
                "mean",
            );
            close(
                rate_at_least(&items, t).unwrap(),
                &c["expected"]["rate_at_least"],
                "rate",
            );
        } else {
            let (p, n, k) = (
                i["passing"].as_u64().unwrap(),
                i["replicates"].as_u64().unwrap(),
                i["k"].as_u64().unwrap(),
            );
            close(
                pass_at_k(p, n, k).unwrap(),
                &c["expected"]["pass_at_k"],
                &format!("pass@{k} {p}/{n}"),
            );
        }
    }
}

#[test]
fn pass_at_k_equals_the_any_at_k_estimand() {
    let doc: Value = serde_json::from_str(REFERENCE).unwrap();
    for c in doc["cases"]["estimand_at_k"].as_array().unwrap() {
        let (pos, reps) = (
            c["input"]["positives"].as_u64().unwrap(),
            c["input"]["replicates"].as_u64().unwrap(),
        );
        for k in [1, 2, 3, reps] {
            close(
                pass_at_k(pos, reps, k).unwrap(),
                &c["expected"][format!("any@{k}")],
                &format!("any@{k} {pos}/{reps}"),
            );
        }
    }
}
