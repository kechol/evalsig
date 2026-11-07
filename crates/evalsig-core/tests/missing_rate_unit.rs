//! The missing rate's counting unit is the item (metrics spec §0, §0.1): replicates of
//! the same item add nothing to the count, so the exact bound is the same at one
//! judgment per item and at several.

use evalsig_core::gate::{self, Sided};
use evalsig_core::records::{Judgment, Record};

/// `items` items with `replicates` judgments each; the first `missing` items time out in
/// every replicate.
fn judgments(items: usize, replicates: u32, missing: usize) -> Vec<Judgment> {
    let mut out = Vec::new();
    for i in 0..items {
        for r in 0..replicates {
            let tail = if i < missing {
                r#","missing":{"reason":"timeout"}"#
            } else {
                r#","value":true"#
            };
            let line = format!(
                r#"{{"record":"judgment","item_id":"i{i}","run_id":"r","gen_replicate":{r},"output_type":"binary"{tail}}}"#
            );
            match Record::from_json(&line).unwrap() {
                Record::Judgment(j) => out.push(j),
                _ => unreachable!(),
            }
        }
    }
    out
}

fn upper_bound(js: &[Judgment]) -> f64 {
    let refs: Vec<&Judgment> = js.iter().collect();
    let (k, n) = gate::missing_counts(&refs, |j| j.missing.is_some());
    // One-sided at 1 - 0.05/8, read from the two-sided interval at 1 - 2 * 0.05/8.
    let e = gate::missing_rate(k, n, 1.0 - 2.0 * 0.05 / 8.0, Sided::One).unwrap();
    match e {
        gate::Evidence::Value { hi, .. } => hi,
        _ => unreachable!(),
    }
}

#[test]
fn replicates_add_nothing_to_the_count() {
    let once = upper_bound(&judgments(400, 1, 20));
    let thrice = upper_bound(&judgments(400, 3, 20));
    assert!((once - thrice).abs() < 1e-12, "{once} vs {thrice}");
    // Clopper-Pearson for 20 of 400 items at 99.375% one-sided, from scipy:
    // beta.ppf(0.99375, 21, 380).
    assert!((once - 0.0839).abs() < 5e-4, "{once}");
}

#[test]
fn a_partly_missing_item_counts_its_share() {
    // Two items: one missing in one of its two replicates, one never missing.
    let mut js = judgments(2, 2, 0);
    js[0].value = None;
    js[0].missing = Some(serde_json::from_str(r#"{"reason":"timeout"}"#).unwrap());
    let refs: Vec<&Judgment> = js.iter().collect();
    assert_eq!(
        gate::missing_counts(&refs, |j| j.missing.is_some()),
        (0.5, 2.0)
    );
}
