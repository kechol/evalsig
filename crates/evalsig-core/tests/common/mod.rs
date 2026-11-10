//! Shared access to the reference values and their tolerance, and reports built from
//! record lines.

#![allow(dead_code)]

use std::collections::BTreeMap;

use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Label, Record, Value as RecordValue};
use evalsig_core::report::{self, Report, Sections, Settings};
use evalsig_core::rules::Rule;
use serde_json::Value;

const REFERENCE: &str = include_str!("../../../../tests/fixtures/reference-values.json");

/// Absolute tolerance the reference values require.
pub const TOLERANCE: f64 = 1e-9;

pub fn reference() -> Value {
    serde_json::from_str(REFERENCE).expect("reference values are JSON")
}

pub fn cases(family: &str) -> Vec<Value> {
    reference()["cases"][family]
        .as_array()
        .unwrap_or_else(|| panic!("no family {family}"))
        .clone()
}

pub fn close(got: f64, want: &Value, what: &str) {
    let want = want
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: expected a number, got {want}"));
    assert!((got - want).abs() <= TOLERANCE, "{what}: {got} vs {want}");
}

/// Settings for a report of the default task with `true` as the positive label, 200
/// resamples and seed 1; tests override the fields they need.
pub fn settings() -> Settings {
    Settings {
        task: "default".into(),
        positive: Some(Rule::Equals(RecordValue::Bool(true))),
        decision: None,
        threshold: None,
        estimand: Estimand::SingleShot,
        missing: MissingPolicy::default(),
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
    }
}

/// The report of the run among JSON record `lines`, with their items and labels.
pub fn report_of(lines: &[String], settings: &Settings) -> Report {
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
        .expect("a run record");
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
    report::build(run, &judgments, &items, &labels, settings).unwrap()
}
