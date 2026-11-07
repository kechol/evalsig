//! Scaling every item weight changes no inference (metrics spec §2, §3.4, §5): d′, c and
//! the feature likelihood ratios are read on the scale of effective items, and the Brier
//! score is a weighted mean.

use std::collections::BTreeMap;

use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Label, Record, Value};
use evalsig_core::report::{self, Report, Sections, Settings};
use evalsig_core::roc;
use evalsig_core::rules::Rule;

/// 20 positives and 20 negatives, every item of weight `w`. The system hits 15 positives
/// and false-alarms on 5 negatives; a rule fires on 12 positives and 6 negatives.
fn report_at(w: f64) -> Report {
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for i in 0..40 {
        let positive = i < 20;
        let k = if positive { i } else { i - 20 };
        let judged = if positive { k < 15 } else { k < 5 };
        let fired = if positive { k < 12 } else { k < 6 };
        lines.push(format!(
            r#"{{"record":"item","item_id":"t{i}","weight":{w}}}"#
        ));
        lines.push(format!(
            r#"{{"record":"label","item_id":"t{i}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
        ));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"t{i}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{judged},"features":[{{"feature_id":"rule.a","fired":{fired}}}]}}"#
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
            features: true,
            ..Sections::default()
        },
        exact_rates: false,
        proxy: None,
    };
    report::build(run, &judgments, &items, &labels, &settings).unwrap()
}

#[test]
fn d_prime_and_the_features_do_not_depend_on_the_scale_of_the_weights() {
    let unit = report_at(1.0);
    let sdt = unit.sdt.as_ref().unwrap();
    let feature = &unit.features.as_ref().unwrap()[0];
    for w in [0.01, 0.1, 2.0, 10.0, 100.0] {
        let r = report_at(w);
        let s = r.sdt.as_ref().unwrap();
        assert!((s.dprime - sdt.dprime).abs() < 1e-9, "d' at w = {w}");
        assert!((s.criterion - sdt.criterion).abs() < 1e-9, "c at w = {w}");
        let f = &r.features.as_ref().unwrap()[0];
        assert!((f.lr - feature.lr).abs() < 1e-9, "lr at w = {w}");
        assert!((f.ci.0 - feature.ci.0).abs() < 1e-9, "ci at w = {w}");
        assert!((f.ci.1 - feature.ci.1).abs() < 1e-9, "ci at w = {w}");
        assert!((f.p_value - feature.p_value).abs() < 1e-9, "p at w = {w}");
        assert_eq!(f.class, feature.class, "class at w = {w}");
    }
}

#[test]
fn the_brier_score_is_a_weighted_mean() {
    let (brier, bins) = roc::calibration(&[0.2, 0.8], &[0.0, 0.0], &[1.0, 3.0]).unwrap();
    // (1 * 0.2² + 3 * 0.8²) / 4, not the unweighted mean 0.34.
    assert!((brier - 0.49).abs() < 1e-12, "{brier}");
    let bin = bins.iter().find(|b| b.bin == 8).unwrap();
    assert_eq!(bin.n, 1);
}
