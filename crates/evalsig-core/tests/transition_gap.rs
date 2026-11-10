//! The transition table (metrics spec §7) fills a system-side gap as accuracy does
//! (§0.1.1), so its cells reconcile with the differences printed beside it.

use std::collections::{BTreeMap, BTreeSet};

use evalsig_core::compare::{self, Baseline, Options};
use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::missing::MissingPolicy;
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

/// Ten positives `p0`..`p9` and ten negatives `n0`..`n9`, each labelled. The run judges
/// every item right, except that it times out on the negatives `n0`..`n{timeouts - 1}`.
fn run(id: &str, timeouts: usize) -> Vec<Record> {
    let mut lines = vec![format!(
        r#"{{"record":"run","run_id":"{id}","system":{{"id":"s","version":"{id}"}},"output_type":"binary"}}"#
    )];
    for (prefix, positive) in [("p", true), ("n", false)] {
        for i in 0..10 {
            let item = format!("{prefix}{i}");
            lines.push(format!(r#"{{"record":"item","item_id":"{item}"}}"#));
            lines.push(format!(
                r#"{{"record":"label","item_id":"{item}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
            ));
            lines.push(if !positive && i < timeouts {
                format!(
                    r#"{{"record":"judgment","item_id":"{item}","run_id":"{id}","gen_replicate":0,"output_type":"binary","missing":{{"reason":"timeout"}}}}"#
                )
            } else {
                format!(
                    r#"{{"record":"judgment","item_id":"{item}","run_id":"{id}","gen_replicate":0,"output_type":"binary","value":{positive}}}"#
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
fn a_candidate_gap_on_negatives_is_a_new_false_alarm() {
    let (a, b) = (run("a", 0), run("b", 4));
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
    let delta = |name: &str| {
        cmp.metrics
            .iter()
            .find(|m| m.metric == name)
            .and_then(|m| m.delta)
            .unwrap()
    };
    // Four of ten negatives become false alarms: the false-alarm rate rises by 0.4 and
    // accuracy falls by 4 of 20 items.
    assert!((delta("false_alarm_rate") - -0.4).abs() < 1e-12);
    assert!((delta("accuracy") - -0.2).abs() < 1e-12);
    let t = cmp.transition.unwrap();
    assert_eq!(
        (
            t.new_hits,
            t.lost_hits,
            t.new_false_alarms,
            t.cleared_false_alarms
        ),
        (0.0, 0.0, 4.0, 0.0)
    );
}
