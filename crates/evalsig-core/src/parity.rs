//! The alternative annotator test (metrics spec §9.4, D-017; Calderon, Reichart and Dror,
//! ACL 2025): could the system replace one of the human raters?
//!
//! For each human rater j, on the items j and at least one other human labelled, the
//! system and j are each scored by their agreement with the other humans. The system
//! wins against j when a one-sided t-test rejects "the system is worse by ε or more";
//! Benjamini-Yekutieli controls the raters tested together.

use std::collections::BTreeMap;

use serde::Serialize;
use statrs::distribution::{ContinuousCDF, StudentsT};

use crate::numeric::{mean, sum, variance};
use crate::records::{Label, LabelKind, Source};
use crate::report::Prepared;

/// The default ε, the paper's value for crowd raters and the strictest of its three.
pub const EPSILON: f64 = 0.1;
/// The false discovery rate over the raters tested, outside a gate.
pub const Q: f64 = 0.05;
/// Raters sharing fewer items with the other humans are not tested.
pub const MIN_ITEMS: usize = 30;

/// How agreement with the other humans is scored on one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    /// The share of the other humans whose label is the same.
    Nominal,
    /// Minus the root-mean-square difference from the other humans' labels.
    Graded,
}

/// The test against one human rater.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RaterTest {
    /// The human rater the system is set against.
    pub rater: String,
    /// Items the rater and at least one other human labelled, with a system value.
    pub items: usize,
    /// Mean of W_j − W_sys.
    pub mean_difference: f64,
    /// The one-sided p-value of H0: mean difference ≥ ε.
    pub p_value: f64,
    /// The Benjamini-Yekutieli adjusted p-value over the tested raters.
    pub adjusted_p_value: f64,
    /// Whether the system wins against this rater.
    pub system_wins: bool,
    /// Mean of W_sys: how often the system agrees with the others at least as well.
    pub advantage: f64,
}

/// The verdict of the test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParityVerdict {
    /// The system wins against at least half of the tested raters.
    HumanParity,
    /// It wins against fewer than half.
    BelowHuman,
    /// Fewer than two raters could be tested.
    InsufficientData,
}

/// The alternative annotator test over every human rater.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Parity {
    /// The margin the system may trail a human by.
    pub epsilon: f64,
    /// The false discovery rate.
    pub q: f64,
    /// The raters tested.
    pub tested: Vec<RaterTest>,
    /// Raters left out, with the items they share with other humans.
    pub skipped: BTreeMap<String, usize>,
    /// The share of tested raters the system wins against (ω).
    pub winning_rate: Option<f64>,
    /// The mean advantage over the tested raters (ρ), for choosing between systems.
    pub advantage_probability: Option<f64>,
    /// The verdict.
    pub verdict: ParityVerdict,
}

/// A run against the human raters among `labels` (their latest first label replicate;
/// adjudications are not raters). A run with a decision rule is compared on decisions,
/// nominally, positive when most of its replicates are, with the labels read by the
/// positive definition; one without, on its values, graded.
pub fn of_run(prepared: &Prepared<'_>, labels: &[&Label], epsilon: f64, q: f64) -> Parity {
    let scale = match (prepared.rule(), prepared.positive()) {
        (Some(_), Some(_)) => Scale::Nominal,
        _ => Scale::Graded,
    };
    let positive = prepared.positive();
    let mut humans: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for l in labels {
        if l.source != Source::Human
            || l.kind() == LabelKind::Adjudication
            || l.label_replicate() != 0
        {
            continue;
        }
        let value = match (scale, positive, l.value) {
            (Scale::Nominal, Some(p), Some(v)) => p.is_positive(v).map(|b| f64::from(u8::from(b))),
            (Scale::Graded, _, Some(v)) => Some(v.as_f64()),
            _ => None,
        };
        if let Some(v) = value {
            humans
                .entry(l.rater_id.clone())
                .or_default()
                .insert(l.item_id.clone(), v);
        }
    }
    let system = prepared
        .judged()
        .into_iter()
        .map(|(item, (v, _))| match scale {
            Scale::Nominal => (item, f64::from(u8::from(v >= 0.5))),
            Scale::Graded => (item, v),
        })
        .collect();
    test(&humans, &system, scale, epsilon, q)
}

/// Each labeller's values by item: `humans[rater][item]`, `system[item]`.
pub fn test(
    humans: &BTreeMap<String, BTreeMap<String, f64>>,
    system: &BTreeMap<String, f64>,
    scale: Scale,
    epsilon: f64,
    q: f64,
) -> Parity {
    let score = |x: f64, others: &[f64]| -> f64 {
        match scale {
            Scale::Nominal => {
                others.iter().filter(|o| **o == x).count() as f64 / others.len() as f64
            }
            Scale::Graded => {
                let squares = sum(others.iter().map(|o| (x - o) * (x - o)));
                -(squares / others.len() as f64).sqrt()
            }
        }
    };
    let mut tested = Vec::new();
    let mut skipped = BTreeMap::new();
    for (j, mine) in humans {
        let mut d = Vec::new();
        let mut w_sys = Vec::new();
        for (item, &x_j) in mine {
            let Some(&x_sys) = system.get(item) else {
                continue;
            };
            let others: Vec<f64> = humans
                .iter()
                .filter(|(r, _)| *r != j)
                .filter_map(|(_, v)| v.get(item).copied())
                .collect();
            if others.is_empty() {
                continue;
            }
            let (s_j, s_sys) = (score(x_j, &others), score(x_sys, &others));
            let (wj, ws) = (
                f64::from(u8::from(s_j >= s_sys)),
                f64::from(u8::from(s_sys >= s_j)),
            );
            d.push(wj - ws);
            w_sys.push(ws);
        }
        if d.len() < MIN_ITEMS {
            skipped.insert(j.clone(), d.len());
            continue;
        }
        let m = mean(d.iter().copied()).expect("items exist");
        tested.push(RaterTest {
            rater: j.clone(),
            items: d.len(),
            mean_difference: m,
            p_value: one_sided_p(&d, m, epsilon),
            adjusted_p_value: f64::NAN,
            system_wins: false,
            advantage: mean(w_sys.iter().copied()).expect("items exist"),
        });
    }
    let adjusted = benjamini_yekutieli(&tested.iter().map(|t| t.p_value).collect::<Vec<_>>());
    for (t, a) in tested.iter_mut().zip(adjusted) {
        t.adjusted_p_value = a;
        t.system_wins = a <= q;
    }
    let (winning_rate, advantage_probability) = if tested.is_empty() {
        (None, None)
    } else {
        let n = tested.len() as f64;
        (
            Some(tested.iter().filter(|t| t.system_wins).count() as f64 / n),
            mean(tested.iter().map(|t| t.advantage)),
        )
    };
    let verdict = match winning_rate {
        _ if tested.len() < 2 => ParityVerdict::InsufficientData,
        Some(w) if w >= 0.5 => ParityVerdict::HumanParity,
        _ => ParityVerdict::BelowHuman,
    };
    Parity {
        epsilon,
        q,
        tested,
        skipped,
        winning_rate,
        advantage_probability,
        verdict,
    }
}

/// P(T ≤ t) for t = (mean − ε) / (s / √n) with n − 1 degrees of freedom. With no
/// spread the mean decides alone.
fn one_sided_p(d: &[f64], m: f64, epsilon: f64) -> f64 {
    let n = d.len() as f64;
    let sd = variance(d, 1).sqrt();
    if sd == 0.0 {
        return if m < epsilon { 0.0 } else { 1.0 };
    }
    let t = (m - epsilon) / (sd / n.sqrt());
    StudentsT::new(0.0, 1.0, n - 1.0)
        .expect("at least MIN_ITEMS items")
        .cdf(t)
}

/// Benjamini-Yekutieli adjusted p-values, in the input order.
fn benjamini_yekutieli(p: &[f64]) -> Vec<f64> {
    let m = p.len();
    let c: f64 = sum((1..=m).map(|i| 1.0 / i as f64));
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|a, b| p[*a].total_cmp(&p[*b]));
    let mut out = vec![0.0; m];
    let mut running = 1.0_f64;
    for (rank, &i) in order.iter().enumerate().rev() {
        let value = (p[i] * m as f64 * c / (rank + 1) as f64).min(1.0);
        running = running.min(value);
        out[i] = running;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjustment_keeps_order_and_caps_at_one() {
        let a = benjamini_yekutieli(&[0.01, 0.04, 0.5]);
        let c = 1.0 + 0.5 + 1.0 / 3.0;
        assert!((a[0] - 0.01 * 3.0 * c).abs() < 1e-12);
        assert!(a[0] <= a[1] && a[1] <= a[2] && a[2] <= 1.0);
    }

    #[test]
    fn one_tested_rater_is_not_enough() {
        let humans: BTreeMap<String, BTreeMap<String, f64>> = [("h1", 1.0), ("h2", 1.0)]
            .into_iter()
            .map(|(r, v)| {
                let items = (0..40).map(|i| (format!("i{i}"), v)).collect();
                (r.to_owned(), items)
            })
            .collect();
        let mut short = humans.clone();
        short.get_mut("h2").unwrap().retain(|k, _| k.len() < 3);
        let system = (0..40).map(|i| (format!("i{i}"), 1.0)).collect();
        // h2 labelled 10 items, so neither rater shares 30 with another human.
        let p = test(&short, &system, Scale::Nominal, EPSILON, Q);
        assert_eq!(p.verdict, ParityVerdict::InsufficientData);
        assert!(p.tested.is_empty());
        assert_eq!((p.skipped["h1"], p.skipped["h2"]), (10, 10));
    }
}
