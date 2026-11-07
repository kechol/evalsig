//! Items left out of an @k estimand (metrics spec §6.1): an item needs k replicates
//! after the missing policy has dropped the ones it drops.

use std::collections::BTreeMap;

use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Label, Record, Value};
use evalsig_core::report::{self, Sections, Settings};
use evalsig_core::rules::Rule;

/// Ten items with three generation replicates each; on the first four the third
/// replicate is a failed grading, which the default policy drops.
fn records() -> Vec<Record> {
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for i in 0..10 {
        lines.push(format!(r#"{{"record":"item","item_id":"t{i}"}}"#));
        lines.push(format!(
            r#"{{"record":"label","item_id":"t{i}","rater_id":"ana","source":"human","value":true,"version":1}}"#
        ));
        for k in 0..3 {
            lines.push(if i < 4 && k == 2 {
                format!(
                    r#"{{"record":"judgment","item_id":"t{i}","run_id":"r","gen_replicate":{k},"output_type":"binary","missing":{{"reason":"grader_failed"}}}}"#
                )
            } else {
                format!(
                    r#"{{"record":"judgment","item_id":"t{i}","run_id":"r","gen_replicate":{k},"output_type":"binary","value":true}}"#
                )
            });
        }
    }
    lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect()
}

#[test]
fn replicates_the_policy_drops_do_not_count_toward_k() {
    let rs = records();
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
    let labels: Vec<&Label> = rs
        .iter()
        .filter_map(|r| match r {
            Record::Label(l) => Some(l),
            _ => None,
        })
        .collect();
    let settings = Settings {
        task: "default".into(),
        positive: Some(Rule::Equals(Value::Bool(true))),
        decision: None,
        threshold: None,
        estimand: Estimand::Any(3),
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
    let r = report::build(run, &judgments, &items, &labels, &settings).unwrap();
    // Four items keep two usable replicates, so any@3 scores the other six.
    let accuracy = r.metrics.iter().find(|m| m.metric == "accuracy").unwrap();
    assert_eq!(accuracy.n, 6.0);
    assert_eq!(r.n_insufficient_replicates, 4);
}
