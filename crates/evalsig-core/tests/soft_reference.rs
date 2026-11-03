//! Each metric under a soft reference against the definitions of metrics spec §9.2,
//! computed by hand for four items with two or three raters.

use std::collections::BTreeMap;

use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::cost;
use evalsig_core::features::{self, Counts};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Label, Record, Run, Value};
use evalsig_core::reference;
use evalsig_core::report::{self, Metric, Settings};
use evalsig_core::roc::{self, SoftItem};
use evalsig_core::rules::Rule;
use evalsig_core::sdt::Confusion;

const TOL: f64 = 1e-12;

/// An item, its raters' labels and the system's judgment.
type Row = (&'static str, &'static [(&'static str, bool)], bool);

const DATA: [Row; 4] = [
    ("q1", &[("ana", true), ("ben", true), ("cy", false)], true),
    ("q2", &[("ana", true), ("ben", false)], true),
    (
        "q3",
        &[("ana", false), ("ben", false), ("cy", false)],
        false,
    ),
    ("q4", &[("ana", true), ("ben", true)], false),
];
/// Hand-computed positive shares s_i.
const SHARES: [f64; 4] = [2.0 / 3.0, 0.5, 0.0, 1.0];

fn records() -> Vec<Record> {
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
        // An adjudication decides only under `single`.
        r#"{"record":"label","item_id":"q2","rater_id":"lead","source":"human","kind":"adjudication","value":true,"version":1}"#
            .to_owned(),
    ];
    for (item, labels, judged) in DATA {
        lines.push(format!(r#"{{"record":"item","item_id":"{item}"}}"#));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"{item}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{judged}}}"#
        ));
        for (rater, v) in labels {
            lines.push(format!(
                r#"{{"record":"label","item_id":"{item}","rater_id":"{rater}","source":"human","value":{v},"version":1}}"#
            ));
        }
    }
    lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect()
}

fn settings(mode: ReferenceMode) -> Settings {
    Settings {
        task: "default".into(),
        positive: Some(Rule::Equals(Value::Bool(true))),
        decision: None,
        threshold: None,
        estimand: Estimand::SingleShot,
        missing: MissingPolicy::default(),
        reference: mode,
        rater_design: RaterDesign::Auto,
        level: 0.95,
        bootstrap: 200,
        seed: 1,
        prevalence: None,
        by: None,
        items: false,
        detail: false,
        features: false,
        proxy: None,
    }
}

struct Parts<'a> {
    run: &'a Run,
    judgments: Vec<&'a Judgment>,
    items: BTreeMap<&'a str, &'a Item>,
    labels: Vec<&'a Label>,
}

fn parts(rs: &[Record]) -> Parts<'_> {
    let mut p = Parts {
        run: rs
            .iter()
            .find_map(|r| match r {
                Record::Run(x) => Some(x),
                _ => None,
            })
            .unwrap(),
        judgments: Vec::new(),
        items: BTreeMap::new(),
        labels: Vec::new(),
    };
    for r in rs {
        match r {
            Record::Judgment(j) => p.judgments.push(j),
            Record::Item(i) => {
                p.items.insert(&i.item_id, i);
            }
            Record::Label(l) => p.labels.push(l),
            _ => {}
        }
    }
    p
}

fn shares(p: &Parts<'_>, mode: ReferenceMode) -> Vec<f64> {
    let positive = Rule::Equals(Value::Bool(true));
    DATA.iter()
        .map(|(item, _, _)| {
            let of_item: Vec<&Label> = p
                .labels
                .iter()
                .copied()
                .filter(|l| l.item_id == *item)
                .collect();
            reference::of(&of_item, &positive, mode).unwrap().share
        })
        .collect()
}

/// The value of the named metric.
fn value_of(metrics: &[Metric], name: &str) -> f64 {
    metrics
        .iter()
        .find(|m| m.metric == name)
        .unwrap()
        .value
        .unwrap()
}

fn judged() -> Vec<f64> {
    DATA.iter().map(|d| f64::from(u8::from(d.2))).collect()
}

#[test]
fn shares_follow_the_mode() {
    let rs = records();
    let p = parts(&rs);
    assert_eq!(shares(&p, ReferenceMode::Soft), SHARES);
    // Majority: q2 ties (the adjudication is not a vote); single: the adjudication.
    assert_eq!(shares(&p, ReferenceMode::Majority), [1.0, 0.5, 0.0, 1.0]);
    assert_eq!(shares(&p, ReferenceMode::Single), [1.0, 1.0, 0.0, 1.0]);
}

#[test]
fn rates_and_accuracy_match_the_soft_definitions() {
    let rs = records();
    let p = parts(&rs);
    let r = report::build(
        p.run,
        &p.judgments,
        &p.items,
        &p.labels,
        &settings(ReferenceMode::Soft),
    )
    .unwrap();
    let metric = |name: &str| value_of(&r.metrics, name);
    let (s, y) = (SHARES, judged());
    let dot = |f: &dyn Fn(usize) -> f64| (0..4).map(f).sum::<f64>();
    // H = Σ s·y / Σ s = 7/13, F = Σ (1−s)·y / Σ (1−s) = 5/11.
    let h = dot(&|i| s[i] * y[i]) / dot(&|i| s[i]);
    let f = dot(&|i| (1.0 - s[i]) * y[i]) / dot(&|i| 1.0 - s[i]);
    assert!((h - 7.0 / 13.0).abs() < TOL && (f - 5.0 / 11.0).abs() < TOL);
    assert!((metric("hit_rate") - h).abs() < TOL);
    assert!((metric("false_alarm_rate") - f).abs() < TOL);
    // Accuracy = mean[y·s + (1−y)(1−s)] = 13/24.
    assert!((metric("accuracy") - 13.0 / 24.0).abs() < TOL);

    // d′ and c come from the same H and F through the weighted confusion counts.
    let mut c = Confusion::default();
    for i in 0..4 {
        c.add_shares(s[i], y[i], 1.0);
    }
    assert!((c.hit_rate().unwrap() - h).abs() < TOL);
    assert!((c.false_alarm_rate().unwrap() - f).abs() < TOL);

    // Expected cost and likelihood ratios count item i as s_i positive, 1 − s_i negative.
    let matrix = [[0.0, 1.0], [5.0, 0.0]];
    let ec = cost::expected_cost(
        &[c.misses, c.hits],
        &[c.correct_rejections, c.false_alarms],
        &matrix.map(Vec::from),
        0.1,
    )
    .unwrap();
    assert!((ec - cost::binary_cost(h, f, &matrix, 0.1)).abs() < TOL);
    let lr = features::likelihood_ratio(
        Counts {
            a: c.hits,
            n_pos: c.positives(),
            b: c.false_alarms,
            n_neg: c.negatives(),
        },
        0.95,
    )
    .unwrap();
    assert!((lr.lr - h / f).abs() < TOL);

    // AUC_soft over all pairs, i = j included with ψ = ½: 155/286.
    let items: Vec<SoftItem> = (0..4)
        .map(|i| SoftItem {
            score: y[i],
            share: s[i],
            weight: 1.0,
        })
        .collect();
    assert!((roc::auc_soft(&items).unwrap() - 155.0 / 286.0).abs() < TOL);

    assert!((reference::accuracy_ceiling(&s).unwrap() - 19.0 / 24.0).abs() < TOL);
}

#[test]
fn fractional_references_send_paired_tests_to_the_bootstrap() {
    let rs = records();
    let p = parts(&rs);
    let independent = |mode| {
        let s = settings(mode);
        report::prepare(p.run, &p.judgments, &p.items, &p.labels, &s)
            .unwrap()
            .independent()
    };
    // McNemar needs 0/1 references; soft shares make the counts weighted.
    assert!(!independent(ReferenceMode::Soft));
    assert!(independent(ReferenceMode::Single));
}

#[test]
fn kappa_vs_reference_averages_kappa_against_each_rater() {
    let system: BTreeMap<&str, bool> = DATA.iter().map(|d| (d.0, d.2)).collect();
    let mut raters: BTreeMap<&str, BTreeMap<&str, bool>> = BTreeMap::new();
    for (item, labels, _) in DATA {
        for (rater, v) in labels {
            raters.entry(rater).or_default().insert(item, *v);
        }
    }
    // ana: po 3/4, pe 1/2 → 0.5; ben: po 1/2, pe 1/2 → 0; cy: po 1/2, pe 1/2 → 0.
    let (mean, each) = reference::kappa_vs_raters(&system, &raters).unwrap();
    assert_eq!(
        each,
        BTreeMap::from([("ana", 0.5), ("ben", 0.0), ("cy", 0.0)])
    );
    assert!((mean - 1.0 / 6.0).abs() < TOL);
}

#[test]
fn items_split_by_agreement_get_their_own_metrics() {
    let rs = records();
    let p = parts(&rs);
    let s = Settings {
        by: Some("reference_agreement".into()),
        ..settings(ReferenceMode::Soft)
    };
    let r = report::build(p.run, &p.judgments, &p.items, &p.labels, &s).unwrap();
    let value = |stratum: &str, name: &str| value_of(&r.by[stratum], name);
    assert_eq!(r.by.keys().collect::<Vec<_>>(), ["split", "unanimous"]);
    // Split: q1 (s 2/3, judged positive) and q2 (s 1/2, positive).
    assert!((value("split", "accuracy") - 7.0 / 12.0).abs() < TOL);
    assert!((value("split", "hit_rate") - 1.0).abs() < TOL);
    // Unanimous: q3 (negative, judged negative) and q4 (positive, judged negative).
    assert!((value("unanimous", "accuracy") - 0.5).abs() < TOL);
    assert!(value("unanimous", "hit_rate").abs() < TOL);
}

#[test]
fn inclusion_probabilities_reweight_partially_verified_items() {
    // Positive judgments are always checked by a human, negative ones half the time.
    // (item, judged, reference, inclusion probability); f and g have no label.
    let rows = [
        ("a", true, Some(true), 1.0),
        ("b", true, Some(false), 1.0),
        ("c", true, Some(true), 1.0),
        ("d", false, Some(true), 0.5),
        ("e", false, Some(false), 0.5),
        ("f", false, None, 0.5),
        ("g", false, None, 0.5),
    ];
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for (item, judged, reference, p) in rows {
        lines.push(format!(
            r#"{{"record":"item","item_id":"{item}","label_inclusion_prob":{p}}}"#
        ));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"{item}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{judged}}}"#
        ));
        if let Some(v) = reference {
            lines.push(format!(
                r#"{{"record":"label","item_id":"{item}","rater_id":"ana","source":"human","value":{v},"version":1}}"#
            ));
        }
    }
    let rs: Vec<Record> = lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect();
    let p = parts(&rs);
    let r = report::build(
        p.run,
        &p.judgments,
        &p.items,
        &p.labels,
        &settings(ReferenceMode::Soft),
    )
    .unwrap();
    let metric = |name: &str| value_of(&r.metrics, name);
    // Weighted by 1/p: H = (1 + 1) / (1 + 1 + 2) and F = 1 / (1 + 2); unweighted they
    // would be 2/3 and 1/2.
    assert!((metric("hit_rate") - 0.5).abs() < TOL);
    assert!((metric("false_alarm_rate") - 1.0 / 3.0).abs() < TOL);
    assert_eq!(r.reference.unwrap().inclusion_weighted_items, 5);
}
