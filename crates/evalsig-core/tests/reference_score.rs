//! Score-type metrics through the report pipeline, and the @k estimands, against the
//! reference values.

use std::collections::BTreeMap;

use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Record};
use evalsig_core::replicates::{Rule, at_k};
use evalsig_core::report::{self, Sections, Settings};
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
    let rs: Vec<Record> = lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect();
    let run = rs
        .iter()
        .find_map(|r| match r {
            Record::Run(x) => Some(x),
            _ => None,
        })
        .unwrap();
    let judgments: Vec<&Judgment> = rs
        .iter()
        .filter_map(|r| match r {
            Record::Judgment(j) => Some(j),
            _ => None,
        })
        .collect();
    let items: BTreeMap<&str, &Item> = rs
        .iter()
        .filter_map(|r| match r {
            Record::Item(i) => Some((i.item_id.as_str(), i)),
            _ => None,
        })
        .collect();
    let settings = Settings {
        task: "default".into(),
        positive: None,
        decision: None,
        threshold: Some(t),
        estimand: Estimand::SingleShot,
        missing: MissingPolicy::default(),
        reference: ReferenceMode::Soft,
        rater_design: RaterDesign::Auto,
        level: 0.95,
        bootstrap: 200,
        seed: 1,
        prevalence: None,
        sections: Sections::default(),
        exact_rates: false,
        proxy: None,
    };
    report::build(run, &judgments, &items, &[], &settings).unwrap()
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
