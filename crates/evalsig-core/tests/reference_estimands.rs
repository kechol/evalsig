//! The @k estimands against the reference values.

use evalsig_core::replicates::{Rule, at_k};
use serde_json::Value;

const REFERENCE: &str = include_str!("../../../tests/fixtures/reference-values.json");

#[test]
fn estimand_at_k_cases_match() {
    let doc: Value = serde_json::from_str(REFERENCE).unwrap();
    let cases = doc["cases"]["estimand_at_k"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    for c in cases {
        let (pos, reps) = (
            c["input"]["positives"].as_u64().unwrap(),
            c["input"]["replicates"].as_u64().unwrap(),
        );
        for (name, rule) in [
            ("majority", Rule::Majority),
            ("any", Rule::Any),
            ("all", Rule::All),
        ] {
            for k in [1, 2, 3, reps] {
                let want = c["expected"][format!("{name}@{k}")].as_f64().unwrap();
                let got = at_k(pos, reps, k, rule).unwrap();
                assert!(
                    (got - want).abs() <= 1e-9,
                    "{name}@{k} {pos}/{reps}: {got} vs {want}"
                );
            }
        }
    }
}
