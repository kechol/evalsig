//! Missing bounds (metrics spec §0.1): when any system-side judgment is missing, the
//! metric also reports the value with those gaps filled worst and filled best, whatever
//! the policy does with them at the point.

use std::collections::BTreeMap;

use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::{MissingPolicy, Policy};
use evalsig_core::records::{Item, Judgment, Label, MissingReason, Record, Value};
use evalsig_core::report::{self, Sections, Settings};
use evalsig_core::rules::Rule;

/// Two positives and two negatives, all judged right except the negative `n2`, whose
/// judgment is an `error`.
fn records() -> Vec<Record> {
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for (item, positive) in [("p1", true), ("p2", true), ("n1", false), ("n2", false)] {
        lines.push(format!(r#"{{"record":"item","item_id":"{item}"}}"#));
        lines.push(format!(
            r#"{{"record":"label","item_id":"{item}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
        ));
        lines.push(if item == "n2" {
            format!(
                r#"{{"record":"judgment","item_id":"{item}","run_id":"r","gen_replicate":0,"output_type":"binary","missing":{{"reason":"error"}}}}"#
            )
        } else {
            format!(
                r#"{{"record":"judgment","item_id":"{item}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{positive}}}"#
            )
        });
    }
    lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect()
}

/// Per metric: the value, its missing bounds and the denominator it used.
type Measured = BTreeMap<String, (Option<f64>, Option<(f64, f64)>, f64)>;

fn bounds_of(missing: MissingPolicy) -> Measured {
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
        estimand: Estimand::SingleShot,
        missing,
        reference: ReferenceMode::Soft,
        rater_design: RaterDesign::Auto,
        level: 0.95,
        bootstrap: 200,
        seed: 1,
        prevalence: None,
        cost: None,
        sections: Sections::default(),
        exact_rates: false,
        proxy: None,
    };
    report::build(run, &judgments, &items, &labels, &settings)
        .unwrap()
        .metrics
        .iter()
        .map(|m| (m.metric.to_owned(), (m.value, m.missing_bounds, m.n)))
        .collect()
}

#[test]
fn a_system_side_reason_on_drop_keeps_the_bounds() {
    let m = bounds_of(MissingPolicy::default().with_override(MissingReason::Error, Policy::Drop));
    // The dropped judgment leaves the point on the three judged items ...
    let (value, bounds, n) = m["accuracy"];
    assert_eq!((value, n), (Some(1.0), 3.0));
    // ... but the bounds fill it worst (judged positive on a negative: 3 of 4 right) and
    // best (4 of 4).
    assert_eq!(bounds, Some((0.75, 1.0)));
    // The false-alarm rate: 0 of 1 judged negative; filled worst 1 of 2, best 0 of 2.
    let (value, bounds, n) = m["false_alarm_rate"];
    assert_eq!((value, n), (Some(0.0), 1.0));
    assert_eq!(bounds, Some((0.0, 0.5)));
}

#[test]
fn the_default_policy_gives_the_same_bounds() {
    let m = bounds_of(MissingPolicy::default());
    let (value, bounds, _) = m["accuracy"];
    assert_eq!((value, bounds), (Some(0.75), Some((0.75, 1.0))));
}
