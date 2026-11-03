//! Replicates, estimands and reproducibility (metrics spec §6).
//!
//! The item is the counting unit. An item with K replicates, c of them positive (or
//! correct), contributes one value whose meaning is set by the estimand.

use crate::config::Estimand;
use crate::interval::clopper_pearson_upper;
use statrs::function::factorial::ln_binomial;

use crate::numeric::{mean, sum};

/// How k replicates are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Positive when more than half are; a tie at even k counts one half.
    Majority,
    /// Positive when any is (pass@k for generation tasks).
    Any,
    /// Positive when all are (pass^k).
    All,
}

/// C(a, j) C(b, k − j) / C(a + b, k): the hypergeometric probability of j positives
/// in k draws from a positives and b negatives, in log space so large replicate
/// counts do not overflow.
fn hypergeometric(a: u64, b: u64, k: u64, j: u64) -> f64 {
    if j > a || k - j > b {
        return 0.0;
    }
    (ln_binomial(a, j) + ln_binomial(b, k - j) - ln_binomial(a + b, k)).exp()
}

/// The probability that a rule over k replicates drawn without replacement from
/// K, c of them positive, says positive (metrics spec §6.1). `None` when k > K.
pub fn at_k(c: u64, big_k: u64, k: u64, rule: Rule) -> Option<f64> {
    if k == 0 || k > big_k || c > big_k {
        return None;
    }
    let (pos, neg) = (c as f64, (big_k - c) as f64);
    let big = big_k as f64;
    Some(match rule {
        // Products of ratios: exact for small counts and free of overflow.
        Rule::Any => {
            1.0 - (0..k)
                .map(|i| (neg - i as f64) / (big - i as f64))
                .product::<f64>()
                .max(0.0)
        }
        Rule::All => (0..k)
            .map(|i| (pos - i as f64) / (big - i as f64))
            .product::<f64>()
            .max(0.0),
        Rule::Majority => sum((0..=k).map(|j| {
            let w = hypergeometric(c, big_k - c, k, j);
            match (2 * j).cmp(&k) {
                std::cmp::Ordering::Greater => w,
                std::cmp::Ordering::Equal => 0.5 * w,
                std::cmp::Ordering::Less => 0.0,
            }
        })),
    })
}

/// An item's value under the estimand: the replicate mean for `single_shot`, the
/// rule's probability for `@k`. `None` when the item has fewer than k replicates;
/// such items are left out and counted as `n_insufficient_replicates`.
pub fn item_value(positives: u64, replicates: u64, estimand: Estimand) -> Option<f64> {
    match estimand {
        Estimand::SingleShot => (replicates > 0 && positives <= replicates)
            .then(|| positives as f64 / replicates as f64),
        Estimand::Majority(k) => at_k(positives, replicates, u64::from(k), Rule::Majority),
        Estimand::Any(k) => at_k(positives, replicates, u64::from(k), Rule::Any),
        Estimand::All(k) => at_k(positives, replicates, u64::from(k), Rule::All),
    }
}

/// Disagreement between replicates of one item: the share of replicate pairs that
/// differ, and whether any pair differs. `None` with fewer than two replicates.
fn item_disagreement<T: PartialEq>(replicates: &[T]) -> Option<(f64, bool)> {
    let k = replicates.len();
    if k < 2 {
        return None;
    }
    let mut differing = 0u64;
    for i in 0..k {
        for j in i + 1..k {
            differing += u64::from(replicates[i] != replicates[j]);
        }
    }
    let pairs = (k * (k - 1) / 2) as f64;
    Some((differing as f64 / pairs, differing > 0))
}

/// Replicate disagreement over items (metrics spec §6.2). Pass generation replicates
/// for system reproducibility, or grading replicates of one output for grader
/// reproducibility; the two are reported separately.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Disagreement {
    /// Mean over items of the share of differing replicate pairs (comparable across
    /// different replicate counts).
    pub pairwise: f64,
    /// Share of items with any differing pair (grows with the replicate count).
    pub any_item: f64,
    /// Items with at least two replicates.
    pub items: u64,
}

impl Disagreement {
    pub fn of<T: PartialEq>(items: &[Vec<T>]) -> Option<Disagreement> {
        let per_item: Vec<(f64, bool)> =
            items.iter().filter_map(|r| item_disagreement(r)).collect();
        Some(Disagreement {
            pairwise: mean(per_item.iter().map(|(p, _)| *p))?,
            any_item: mean(per_item.iter().map(|(_, any)| f64::from(u8::from(*any))))?,
            items: per_item.len() as u64,
        })
    }

    /// One-sided upper bound on the pairwise rate at `level`, by the exact method on
    /// the effective size (items, or clusters when given).
    pub fn upper_bound(&self, effective_n: f64, level: f64) -> f64 {
        clopper_pearson_upper(self.pairwise * effective_n, effective_n, level)
    }
}

/// Run-to-run standard deviation of a metric over R ≥ 2 runs of the same system
/// version: a diagnostic only, never part of an interval (metrics spec §6.3).
pub fn run_to_run_sd(values: &[f64]) -> Option<f64> {
    let r = values.len();
    if r < 2 {
        return None;
    }
    let m = mean(values.iter().copied())?;
    let ss = sum(values.iter().map(|v| (v - m).powi(2)));
    Some((ss / (r - 1) as f64).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same value by enumerating every k-subset of the replicates.
    fn brute(c: u64, big_k: u64, k: u64, rule: Rule) -> f64 {
        let obs: Vec<bool> = (0..big_k).map(|i| i < c).collect();
        let mut total = 0.0;
        let mut count = 0.0;
        for mask in 0u32..(1 << big_k) {
            if u64::from(mask.count_ones()) != k {
                continue;
            }
            let j = (0..big_k)
                .filter(|&i| mask & (1 << i) != 0 && obs[i as usize])
                .count() as u64;
            total += match rule {
                Rule::Any => f64::from(u8::from(j > 0)),
                Rule::All => f64::from(u8::from(j == k)),
                Rule::Majority if 2 * j > k => 1.0,
                Rule::Majority if 2 * j == k => 0.5,
                Rule::Majority => 0.0,
            };
            count += 1.0;
        }
        total / count
    }

    #[test]
    fn closed_forms_match_enumeration() {
        for big_k in 1..=10 {
            for c in 0..=big_k {
                for k in 1..=big_k {
                    for rule in [Rule::Majority, Rule::Any, Rule::All] {
                        let a = at_k(c, big_k, k, rule).unwrap();
                        let b = brute(c, big_k, k, rule);
                        assert!(
                            (a - b).abs() < 1e-12,
                            "{rule:?} c={c} K={big_k} k={k}: {a} vs {b}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn large_replicate_counts_stay_finite() {
        for rule in [Rule::Majority, Rule::Any, Rule::All] {
            let v = at_k(1000, 2000, 1000, rule).unwrap();
            assert!(v.is_finite() && (0.0..=1.0).contains(&v), "{rule:?} {v}");
        }
        assert!((at_k(1000, 2000, 1000, Rule::Majority).unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn estimands_and_insufficient_replicates() {
        assert_eq!(item_value(5, 4, Estimand::SingleShot), None);
        assert_eq!(item_value(2, 4, Estimand::SingleShot), Some(0.5));
        assert_eq!(item_value(2, 4, Estimand::Any(5)), None);
        assert_eq!(item_value(4, 4, Estimand::All(3)), Some(1.0));
    }

    #[test]
    fn disagreement_measures() {
        let items = vec![vec![true, true, false], vec![true, true], vec![false]];
        let d = Disagreement::of(&items).unwrap();
        assert_eq!(d.items, 2);
        assert!((d.pairwise - (2.0 / 3.0) / 2.0).abs() < 1e-15);
        assert_eq!(d.any_item, 0.5);
        assert!(d.upper_bound(2.0, 0.95) > d.pairwise);
        assert_eq!(Disagreement::of::<bool>(&[vec![true]]), None);
    }

    #[test]
    fn run_variance() {
        assert_eq!(run_to_run_sd(&[0.5]), None);
        assert!((run_to_run_sd(&[0.1, 0.2, 0.3]).unwrap() - 0.1).abs() < 1e-15);
    }
}
