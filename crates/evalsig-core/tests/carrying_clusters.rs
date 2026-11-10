//! A bootstrap interval needs at least 5 resampling units that carry the metric's
//! denominator (metrics spec §1.1, §8): ten clusters that hold only three positives, all in
//! one cluster, give the hit rate a single unit, whatever the count of clusters.

use std::collections::{BTreeMap, BTreeSet};

use evalsig_core::compare::{self, Baseline, Options};
use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
use evalsig_core::paired::Verdict;
use evalsig_core::records::{Item, Judgment, Label, Record, Run, Value};
use evalsig_core::report::{self, Sections, Settings};
use evalsig_core::rules::Rule;

fn settings() -> Settings {
    Settings {
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
        cost: None,
        sections: Sections::default(),
        exact_rates: false,
        proxy: None,
    }
}

/// Ten clusters of three items. The three items of cluster 0 are positive and the other
/// 27 negative; the run judges `hits` of the positives positive and every negative right.
fn run(id: &str, hits: usize) -> Vec<Record> {
    let mut lines = vec![format!(
        r#"{{"record":"run","run_id":"{id}","system":{{"id":"s","version":"{id}"}},"output_type":"binary"}}"#
    )];
    for c in 0..10 {
        for k in 0..3 {
            let item = format!("c{c}i{k}");
            let positive = c == 0;
            let judged = positive && k < hits;
            lines.push(format!(
                r#"{{"record":"item","item_id":"{item}","cluster":"c{c}"}}"#
            ));
            lines.push(format!(
                r#"{{"record":"label","item_id":"{item}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
            ));
            lines.push(format!(
                r#"{{"record":"judgment","item_id":"{item}","run_id":"{id}","gen_replicate":0,"output_type":"binary","value":{judged}}}"#
            ));
        }
    }
    lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect()
}

#[test]
fn clusters_without_positives_do_not_count_toward_the_hit_rate() {
    let (a, b) = (run("a", 2), run("b", 1));
    let all: Vec<&Record> = a.iter().chain(&b).collect();
    let items: BTreeMap<&str, &Item> = all
        .iter()
        .filter_map(|r| match r {
            Record::Item(i) => Some((i.item_id.as_str(), i)),
            _ => None,
        })
        .collect();
    let labels: Vec<&Label> = a
        .iter()
        .filter_map(|r| match r {
            Record::Label(l) => Some(l),
            _ => None,
        })
        .collect();
    let arm = |rs: &[Record]| -> (Run, Vec<Judgment>) {
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
    let s = settings();
    let ((ra, ja), (rb, jb)) = (arm(&a), arm(&b));
    let (ja, jb): (Vec<&Judgment>, Vec<&Judgment>) = (ja.iter().collect(), jb.iter().collect());
    let pa = report::prepare(&ra, &ja, &items, &labels, &s).unwrap();
    let pb = report::prepare(&rb, &jb, &items, &labels, &s).unwrap();
    let options = Options {
        level: 0.95,
        bootstrap: 200,
        seed: 1,
        min_effect: BTreeMap::new(),
        excluded: BTreeSet::new(),
        scope: None,
    };
    let cmp = compare::compare(&Baseline::new(&pa), &pb, &options);
    let metric = |name: &str| cmp.metrics.iter().find(|m| m.metric == name).unwrap();
    // The hit rate has one carrying cluster: no interval, not a degenerate one.
    let h = metric("hit_rate");
    assert_eq!(h.interval, "too_few_units");
    assert_eq!(h.ci, None);
    assert_eq!(h.verdict, Verdict::InsufficientData);
    // Accuracy counts every item, so its ten clusters are enough.
    assert_eq!(metric("accuracy").interval, "bootstrap");
}
