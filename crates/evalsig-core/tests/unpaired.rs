//! Unpaired comparison (metrics spec §7.3): Newcombe for 0/1 rates against the
//! literature example, Welch for mean scores, and the drift label.

use std::collections::{BTreeMap, BTreeSet};

use evalsig_core::compare::{self, Options};
use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Label, Record, Run, Value};
use evalsig_core::report::{self, Settings};
use evalsig_core::rules::Rule;
use statrs::distribution::{ContinuousCDF, StudentsT};
mod common;

use common::{cases, close};

fn settings(positive: Option<Rule>) -> Settings {
    Settings {
        task: "default".into(),
        positive,
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
        by: None,
        items: false,
        detail: false,
        features: false,
        exact_rates: false,
        proxy: None,
    }
}

fn options() -> Options {
    Options {
        level: 0.95,
        bootstrap: 200,
        seed: 1,
        min_effect: BTreeMap::new(),
        excluded: BTreeSet::new(),
        scope: None,
    }
}

/// Records of one run on its own items: `values[i]` is the judgment of item
/// `{run}{i}`, labelled positive when `labelled`.
fn run(id: &str, kind: &str, values: &[String], labelled: bool) -> Vec<Record> {
    let semantics = if kind == "score" {
        r#","value_semantics":"score""#
    } else {
        ""
    };
    let mut lines = vec![format!(
        r#"{{"record":"run","run_id":"{id}","system":{{"id":"s","version":"{id}"}}{semantics},"output_type":"{kind}"}}"#
    )];
    for (i, v) in values.iter().enumerate() {
        lines.push(format!(r#"{{"record":"item","item_id":"{id}{i}"}}"#));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"{id}{i}","run_id":"{id}","gen_replicate":0,"output_type":"{kind}","value":{v}}}"#
        ));
        if labelled {
            lines.push(format!(
                r#"{{"record":"label","item_id":"{id}{i}","rater_id":"ana","source":"human","value":true,"version":1}}"#
            ));
        }
    }
    lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect()
}

fn compare_runs(a: &[Record], b: &[Record], positive: Option<Rule>) -> compare::Comparison {
    let all: Vec<&Record> = a.iter().chain(b).collect();
    let items: BTreeMap<&str, &Item> = all
        .iter()
        .filter_map(|r| match r {
            Record::Item(i) => Some((i.item_id.as_str(), i)),
            _ => None,
        })
        .collect();
    let labels: Vec<&Label> = all
        .iter()
        .filter_map(|r| match r {
            Record::Label(l) => Some(l),
            _ => None,
        })
        .collect();
    let judgments = |rs: &[Record]| -> (Run, Vec<Judgment>) {
        let run = rs
            .iter()
            .find_map(|r| match r {
                Record::Run(x) => Some(x.clone()),
                _ => None,
            })
            .unwrap();
        let js = rs
            .iter()
            .filter_map(|r| match r {
                Record::Judgment(j) => Some(j.clone()),
                _ => None,
            })
            .collect();
        (run, js)
    };
    let s = settings(positive);
    let ((ra, ja), (rb, jb)) = (judgments(a), judgments(b));
    let (ja, jb): (Vec<&Judgment>, Vec<&Judgment>) = (ja.iter().collect(), jb.iter().collect());
    let pa = report::prepare(&ra, &ja, &items, &labels, &s).unwrap();
    let pb = report::prepare(&rb, &jb, &items, &labels, &s).unwrap();
    compare::unpaired(&pa, &pb, &options())
}

#[test]
fn zero_one_rates_use_newcombe() {
    // Baseline 48 of 80 correct, candidate 56 of 70: Newcombe (1998) Table II (a).
    let c = &cases("unpaired_difference")[0];
    let bools =
        |k: usize, n: usize| -> Vec<String> { (0..n).map(|i| (i < k).to_string()).collect() };
    let a = run("a", "binary", &bools(48, 80), true);
    let b = run("b", "binary", &bools(56, 70), true);
    let cmp = compare_runs(&a, &b, Some(Rule::Equals(Value::Bool(true))));
    let acc = cmp.metrics.iter().find(|m| m.metric == "accuracy").unwrap();
    assert_eq!(acc.interval, "newcombe");
    assert!((acc.delta.unwrap() - 0.2).abs() < 1e-12);
    close(acc.ci.unwrap().0, &c["expected"]["newcombe95"][0], "lo");
    close(acc.ci.unwrap().1, &c["expected"]["newcombe95"][1], "hi");
    assert_eq!(compare::drift(acc), compare::Drift::Drifted);
}

#[test]
fn mean_scores_use_welch() {
    let xa = [0.2, 0.4, 0.5, 0.3, 0.6, 0.4];
    let xb = [0.5, 0.7, 0.4, 0.6, 0.8, 0.5, 0.6];
    let s = |x: &[f64]| -> Vec<String> { x.iter().map(|v| format!("{v:?}")).collect() };
    let cmp = compare_runs(
        &run("a", "score", &s(&xa), false),
        &run("b", "score", &s(&xb), false),
        None,
    );
    let m = cmp
        .metrics
        .iter()
        .find(|m| m.metric == "mean_score")
        .unwrap();
    assert_eq!(m.interval, "welch");
    // Welch by hand.
    let mean = |x: &[f64]| x.iter().sum::<f64>() / x.len() as f64;
    let var = |x: &[f64]| {
        let m = mean(x);
        x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (x.len() - 1) as f64
    };
    let (va, vb) = (var(&xa) / 6.0, var(&xb) / 7.0);
    let df = (va + vb).powi(2) / (va * va / 5.0 + vb * vb / 6.0);
    let q = StudentsT::new(0.0, 1.0, df).unwrap().inverse_cdf(0.975);
    let d = mean(&xb) - mean(&xa);
    let (lo, hi) = m.ci.unwrap();
    assert!((m.delta.unwrap() - d).abs() < 1e-12);
    assert!((lo - (d - q * (va + vb).sqrt())).abs() < 1e-12);
    assert!((hi - (d + q * (va + vb).sqrt())).abs() < 1e-12);
}
