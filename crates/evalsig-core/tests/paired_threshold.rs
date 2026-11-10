//! Paired comparison of a score run read at a threshold (metrics spec §1.1, §7): the
//! share at or above t is a rate of 0/1 items, so it takes Tango's interval and score test
//! like accuracy, although a score run has no decision rule.

use std::collections::{BTreeMap, BTreeSet};

use evalsig_core::compare::{self, Baseline, Options};
use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Record, Run};
use evalsig_core::report::{self, Sections, Settings};

fn settings() -> Settings {
    Settings {
        task: "default".into(),
        positive: None,
        decision: None,
        threshold: Some(0.5),
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

/// Records of a score run over items `i0`, `i1`, … with the given scores.
fn run(id: &str, scores: &[f64]) -> Vec<Record> {
    let mut lines = vec![format!(
        r#"{{"record":"run","run_id":"{id}","system":{{"id":"s","version":"{id}"}},"value_semantics":"score","output_type":"score"}}"#
    )];
    for (i, v) in scores.iter().enumerate() {
        lines.push(format!(r#"{{"record":"item","item_id":"i{i}"}}"#));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"i{i}","run_id":"{id}","gen_replicate":0,"output_type":"score","value":{v}}}"#
        ));
    }
    lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect()
}

fn parts(records: &[Record]) -> (Run, Vec<Judgment>, Vec<Item>) {
    let run = records
        .iter()
        .find_map(|r| match r {
            Record::Run(x) => Some(x.clone()),
            _ => None,
        })
        .unwrap();
    let judgments = records
        .iter()
        .filter_map(|r| match r {
            Record::Judgment(j) => Some(j.clone()),
            _ => None,
        })
        .collect();
    let items = records
        .iter()
        .filter_map(|r| match r {
            Record::Item(i) => Some(i.clone()),
            _ => None,
        })
        .collect();
    (run, judgments, items)
}

#[test]
fn rate_at_least_takes_tangos_interval_and_score_test() {
    // Baseline 5 of 8 at or above 0.5, candidate 7 of 8: two items only the candidate
    // passes, none only the baseline.
    let a = run("a", &[0.9, 0.9, 0.9, 0.9, 0.9, 0.1, 0.1, 0.1]);
    let b = run("b", &[0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.1]);
    let (ra, ja, items) = parts(&a);
    let (rb, jb, _) = parts(&b);
    let items: BTreeMap<&str, &Item> = items.iter().map(|i| (i.item_id.as_str(), i)).collect();
    let (ja, jb): (Vec<&Judgment>, Vec<&Judgment>) = (ja.iter().collect(), jb.iter().collect());
    let s = settings();
    let pa = report::prepare(&ra, &ja, &items, &[], &s).unwrap();
    let pb = report::prepare(&rb, &jb, &items, &[], &s).unwrap();
    let options = Options {
        level: 0.95,
        bootstrap: 200,
        seed: 1,
        min_effect: BTreeMap::new(),
        excluded: BTreeSet::new(),
        scope: None,
    };
    let cmp = compare::compare(&Baseline::new(&pa), &pb, &options);
    let m = cmp
        .metrics
        .iter()
        .find(|m| m.metric == "rate_at_least")
        .unwrap();
    assert_eq!(m.interval, "tango");
    assert_eq!(m.p_method, "tango_score");
    assert!((m.delta.unwrap() - 0.25).abs() < 1e-12);
    // 2(1 − Φ(2 / √2)).
    assert!((m.p_value.unwrap() - 0.157299).abs() < 1e-6);
    // Tango's score interval for b = 2, c = 0, n = 8 at 95%, solved independently
    // with scipy's brentq on the constrained-MLE score statistic.
    let (lo, hi) = m.ci.unwrap();
    assert!((lo - -0.155509).abs() < 1e-5, "{lo}");
    assert!((hi - 0.590725).abs() < 1e-5, "{hi}");
}

#[test]
fn a_baseline_for_one_metric_compares_only_that_metric() {
    let a = run("a", &[0.9, 0.9, 0.9, 0.9, 0.9, 0.1, 0.1, 0.1]);
    let b = run("b", &[0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.9, 0.1]);
    let (ra, ja, items) = parts(&a);
    let (rb, jb, _) = parts(&b);
    let items: BTreeMap<&str, &Item> = items.iter().map(|i| (i.item_id.as_str(), i)).collect();
    let (ja, jb): (Vec<&Judgment>, Vec<&Judgment>) = (ja.iter().collect(), jb.iter().collect());
    let s = settings();
    let pa = report::prepare(&ra, &ja, &items, &[], &s).unwrap();
    let pb = report::prepare(&rb, &jb, &items, &[], &s).unwrap();
    let options = Options {
        level: 0.95,
        bootstrap: 200,
        seed: 1,
        min_effect: BTreeMap::new(),
        excluded: BTreeSet::new(),
        scope: None,
    };
    let full = compare::compare(&Baseline::new(&pa), &pb, &options);
    let only = Baseline::only(&pa, "rate_at_least");
    let one = compare::compare(&only, &pb, &options);
    assert!(full.metrics.len() > 1);
    assert_eq!(one.metrics.len(), 1);
    // The same metric, with the same draws, whatever else the baseline holds.
    let same = full.metrics.iter().find(|m| m.metric == "rate_at_least");
    assert_eq!(one.metrics.first(), same);
    assert!(
        compare::sequential(&only, &pb, &options)
            .iter()
            .all(|d| d.metric == "rate_at_least")
    );
}
