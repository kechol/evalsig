//! Score-type metrics through the report pipeline, and the @k estimands, against the
//! reference values.

use evalsig_core::replicates::{Rule, at_k};
use evalsig_core::report::{self, Settings};
mod common;

use common::{cases, close};

/// The report of a score run with one judgment per item, read at threshold `t`.
fn report_of(scores: &[f64], weights: &[f64], t: f64) -> report::Report {
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"value_semantics":"score","output_type":"score"}"#
            .to_owned(),
    ];
    for (i, (s, w)) in scores.iter().zip(weights).enumerate() {
        lines.push(format!(
            r#"{{"record":"item","item_id":"i{i}","weight":{w}}}"#
        ));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"i{i}","run_id":"r","gen_replicate":0,"output_type":"score","value":{s}}}"#
        ));
    }
    common::report_of(
        &lines,
        &Settings {
            positive: None,
            threshold: Some(t),
            ..common::settings()
        },
    )
}

#[test]
fn score_cases_match() {
    let cases = cases("score");
    assert_eq!(cases.len(), 8);
    for c in &cases {
        let i = &c["input"];
        let f = |v: &serde_json::Value| -> Vec<f64> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        };
        if i["scores"].is_array() {
            let r = report_of(
                &f(&i["scores"]),
                &f(&i["weights"]),
                i["t"].as_f64().unwrap(),
            );
            let value = |name: &str| {
                r.metrics
                    .iter()
                    .find(|m| m.metric == name)
                    .and_then(|m| m.value)
                    .unwrap()
            };
            close(value("mean_score"), &c["expected"]["mean_score"], "mean");
            close(
                value("rate_at_least"),
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
                at_k(p, n, k, Rule::Any).unwrap(),
                &c["expected"]["pass_at_k"],
                &format!("pass@{k} {p}/{n}"),
            );
        }
    }
}
