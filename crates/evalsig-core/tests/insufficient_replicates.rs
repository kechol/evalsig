//! Items left out of an @k estimand (metrics spec §6.1): an item needs k replicates
//! after the missing policy has dropped the ones it drops.

use evalsig_core::config::Estimand;
use evalsig_core::report::{self, Settings};
mod common;

/// Ten items with three generation replicates each; on the first four the third
/// replicate is a failed grading, which the default policy drops.
fn lines() -> Vec<String> {
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
}

/// The report of `lines()` under an estimand.
fn report_with(estimand: Estimand) -> report::Report {
    common::report_of(
        &lines(),
        &Settings {
            estimand,
            ..common::settings()
        },
    )
}

#[test]
fn replicates_the_policy_drops_do_not_count_toward_k() {
    let r = report_with(Estimand::Any(3));
    // Four items keep two usable replicates, so any@3 scores the other six.
    let accuracy = r.metrics.iter().find(|m| m.metric == "accuracy").unwrap();
    assert_eq!(accuracy.n, 6.0);
    assert_eq!(r.n_insufficient_replicates, 4);
}

#[test]
fn a_rate_of_one_keeps_an_interval_of_width() {
    // Every replicate is right, so every bootstrap resample would give 1: the exact
    // interval on the effective number of items is shown instead (metrics spec §1).
    let r = report_with(Estimand::SingleShot);
    let accuracy = r.metrics.iter().find(|m| m.metric == "accuracy").unwrap();
    assert_eq!(accuracy.value, Some(1.0));
    assert_eq!(accuracy.interval, "clopper_pearson");
    let (lo, hi) = accuracy.ci.unwrap();
    assert!(lo < 0.75 && hi == 1.0, "{lo} {hi}");
}
