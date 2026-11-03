//! The @k estimands against the reference values.

use evalsig_core::replicates::{Rule, at_k};
mod common;

use common::{cases, close};

#[test]
fn estimand_at_k_cases_match() {
    let cases = cases("estimand_at_k");
    assert_eq!(cases.len(), 6);
    for c in &cases {
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
                let got = at_k(pos, reps, k, rule).unwrap();
                close(
                    got,
                    &c["expected"][format!("{name}@{k}")],
                    &format!("{name}@{k} {pos}/{reps}"),
                );
            }
        }
    }
}
