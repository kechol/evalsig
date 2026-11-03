//! Score-type metrics and the @k estimands against the reference values.

use evalsig_core::score::{Scored, mean_score, pass_at_k, rate_at_least};
mod common;

use common::{cases, close};

#[test]
fn score_cases_match() {
    let cases = cases("score");
    assert_eq!(cases.len(), 8);
    for c in &cases {
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
