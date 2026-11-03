//! Pairwise preference: Bradley-Terry against the reference values, and the
//! `position_bias` / `no_swapped_pairs` checks on triggering and quiet fixtures.

use evalsig_core::checks::Overrides;
use evalsig_core::preference::{self, Comparison, RankingError};
use evalsig_core::records::{Preference, Record};
mod common;

use common::{cases, close};

#[test]
fn bradley_terry_cases_match() {
    let cases = cases("bradley_terry");
    assert_eq!(cases.len(), 2);
    for c in &cases {
        let arms = c["input"]["arms"].as_u64().unwrap() as usize;
        let comparisons: Vec<Comparison> = c["input"]["comparisons"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Comparison {
                i: v[0].as_u64().unwrap() as usize,
                j: v[1].as_u64().unwrap() as usize,
                score: v[2].as_f64().unwrap(),
            })
            .collect();
        let fit = preference::bradley_terry(arms, &comparisons).unwrap();
        for (k, s) in fit.log_strength.iter().enumerate() {
            close(*s, &c["expected"]["log_strength"][k], "log strength");
        }
        match fit.tie {
            Some(nu) => close(nu, &c["expected"]["tie"], "tie"),
            None => assert!(c["expected"]["tie"].is_null()),
        }
    }
}

#[test]
fn rankings_need_a_connected_estimable_graph() {
    let c = |i, j, score| Comparison { i, j, score };
    // Arms 0-1 and 2-3 are never compared with each other.
    assert_eq!(
        preference::bradley_terry(4, &[c(0, 1, 1.0), c(1, 0, 1.0), c(2, 3, 0.5)]),
        Err(RankingError::Disconnected)
    );
    // Arm 1 wins every comparison: the maximum lies at infinity.
    assert_eq!(
        preference::bradley_terry(2, &[c(0, 1, 1.0), c(1, 0, 0.0)]),
        Err(RankingError::NotEstimable)
    );
}

/// Preferences of runs `a` and `b` on items i0…: (a shown left, outcome).
fn prefs(rows: &[(bool, &str)]) -> Vec<Preference> {
    rows.iter()
        .enumerate()
        .map(|(k, (a_left, outcome))| {
            let (l, r) = if *a_left { ("a", "b") } else { ("b", "a") };
            let json = format!(
                r#"{{"record":"preference","item_id":"i{}","left_run_id":"{l}","right_run_id":"{r}","rater_id":"ana","source":"human","outcome":"{outcome}"}}"#,
                k / 2
            );
            match Record::from_json(&json).unwrap() {
                Record::Preference(p) => p,
                _ => unreachable!(),
            }
        })
        .collect()
}

fn codes(p: &[Preference]) -> Vec<&'static str> {
    let judged = preference::normalise(p, "a", "b");
    let bias = preference::position_bias(&judged, 0.95);
    preference::checks(&bias, &Overrides::default())
        .into_iter()
        .map(|f| f.code)
        .collect()
}

#[test]
fn the_left_side_winning_swapped_pairs_triggers_position_bias() {
    // Twenty items, each judged in both orders, and the left side always wins.
    let rows: Vec<(bool, &str)> = (0..40).map(|k| (k % 2 == 0, "left")).collect();
    let p = prefs(&rows);
    assert_eq!(codes(&p), ["position_bias"]);
    let judged = preference::normalise(&p, "a", "b");
    let bias = preference::position_bias(&judged, 0.95);
    assert_eq!(
        (bias.swapped_pairs, bias.order_consistency),
        (20, Some(0.0))
    );
    // Swaps cancel: each item's mean is ½, so the win rate is ½.
    let w = preference::win_rate(&judged, 0.95, 200, 1).unwrap();
    assert_eq!((w.value, w.items, w.swapped_pairs), (0.5, 20, 20));
}

#[test]
fn consistent_swapped_pairs_raise_no_check() {
    // B wins in both orders on every item: the left side wins half the time.
    let rows: Vec<(bool, &str)> = (0..40)
        .map(|k| {
            let a_left = k % 2 == 0;
            (a_left, if a_left { "right" } else { "left" })
        })
        .collect();
    let p = prefs(&rows);
    assert!(codes(&p).is_empty());
    let judged = preference::normalise(&p, "a", "b");
    assert_eq!(
        preference::win_rate(&judged, 0.95, 200, 1).unwrap().value,
        1.0
    );
    assert_eq!(
        preference::position_bias(&judged, 0.95).order_consistency,
        Some(1.0)
    );
}

#[test]
fn judgments_in_one_order_only_trigger_no_swapped_pairs() {
    let rows: Vec<(bool, &str)> = (0..10)
        .map(|k| (true, if k % 3 == 0 { "tie" } else { "right" }))
        .collect();
    // Item ids pair rows two by two, but every row shows A on the left.
    assert_eq!(codes(&prefs(&rows)), ["no_swapped_pairs"]);
}

#[test]
fn raters_agree_on_normalised_verdicts() {
    let mut p = prefs(&[
        (true, "right"),
        (false, "left"),
        (true, "left"),
        (false, "right"),
    ]);
    let mut other = p.clone();
    for q in &mut other {
        q.rater_id = "ben".into();
    }
    p.extend(other);
    let judged = preference::normalise(&p, "a", "b");
    let pairs = preference::rater_agreement(&judged);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].items, 2);
    assert_eq!(pairs[0].cohen_kappa, Some(1.0));
}

#[test]
fn three_arms_are_ranked_with_intervals() {
    // c beats b, b beats a, mostly; ties now and then. Ten items, three judgments each.
    let mut lines = Vec::new();
    for item in 0..10 {
        for (l, r, o) in [
            ("a", "b", if item % 4 == 0 { "left" } else { "right" }),
            ("b", "c", if item % 5 == 0 { "tie" } else { "right" }),
            ("c", "a", if item % 3 == 0 { "right" } else { "left" }),
        ] {
            lines.push(format!(
                r#"{{"record":"preference","item_id":"i{item}","left_run_id":"{l}","right_run_id":"{r}","rater_id":"ana","source":"human","outcome":"{o}"}}"#
            ));
        }
    }
    let prefs: Vec<Preference> = lines
        .iter()
        .map(|l| match Record::from_json(l).unwrap() {
            Record::Preference(p) => p,
            _ => unreachable!(),
        })
        .collect();
    let r = preference::ranking(&prefs, 0.95, 200, 7).unwrap();
    let order: Vec<&str> = r.arms.iter().map(|a| a.arm.as_str()).collect();
    assert_eq!(order, ["c", "b", "a"]);
    assert_eq!(r.arms[0].rank, 1);
    assert!(r.tie.is_some());
    assert_eq!(r.interval, "bootstrap");
    let (lo, hi) = r.arms[0].ci.unwrap();
    assert!(lo <= r.arms[0].log_strength && r.arms[0].log_strength <= hi);
}
