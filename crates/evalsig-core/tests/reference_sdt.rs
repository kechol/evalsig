//! Binary signal-detection metrics against the reference values (absolute 1e-9): the
//! raw rates, d′ and c through the report pipeline, the corrected rates from the counts.

use std::collections::BTreeMap;

use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Label, Record, Value};
use evalsig_core::report::{self, Report, Sections, Settings};
use evalsig_core::rules::Rule;
use evalsig_core::sdt::Confusion;
mod common;

use common::{cases, close};

/// A binary run with `k_hit` of `n_pos` positives and `k_fa` of `n_neg` negatives
/// judged positive.
fn report_of(k_hit: usize, n_pos: usize, k_fa: usize, n_neg: usize) -> Report {
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for i in 0..n_pos + n_neg {
        let positive = i < n_pos;
        let judged = if positive {
            i < k_hit
        } else {
            i - n_pos < k_fa
        };
        lines.push(format!(r#"{{"record":"item","item_id":"t{i}"}}"#));
        lines.push(format!(
            r#"{{"record":"label","item_id":"t{i}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
        ));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"t{i}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{judged}}}"#
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
        missing: MissingPolicy::default(),
        reference: ReferenceMode::Soft,
        rater_design: RaterDesign::Auto,
        level: 0.95,
        bootstrap: 200,
        seed: 1,
        prevalence: None,
        sections: Sections {
            detail: true,
            ..Sections::default()
        },
        exact_rates: false,
        proxy: None,
    };
    report::build(run, &judgments, &items, &labels, &settings).unwrap()
}

#[test]
fn sdt_binary_cases_match() {
    let cases = cases("sdt_binary");
    assert_eq!(cases.len(), 4);
    for c in &cases {
        let i = &c["input"];
        let n = |k: &str| i[k].as_u64().unwrap() as usize;
        let (kh, np, kf, nn) = (n("k_hit"), n("n_pos"), n("k_fa"), n("n_neg"));
        let r = report_of(kh, np, kf, nn);
        let metric = |name: &str| {
            r.metrics
                .iter()
                .find(|m| m.metric == name)
                .and_then(|m| m.value)
                .unwrap()
        };
        let sdt = r.sdt.as_ref().unwrap();
        let conf = Confusion {
            hits: kh as f64,
            misses: (np - kh) as f64,
            false_alarms: kf as f64,
            correct_rejections: (nn - kf) as f64,
        };
        let e = &c["expected"];
        for (name, got) in [
            ("H_raw", metric("hit_rate")),
            ("F_raw", metric("false_alarm_rate")),
            ("H_ll", conf.corrected_hit_rate()),
            ("F_ll", conf.corrected_false_alarm_rate()),
            ("dprime", sdt.dprime),
            ("c", sdt.criterion),
        ] {
            close(got, &e[name], name);
        }
    }
}
