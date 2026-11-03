//! Metrics of score-type judgments (`value_semantics = "score"`, metrics spec §0.2):
//! the judgment grades an output, and no reference label is needed.

use crate::numeric::sum;

/// One item's score and weight. With replicates, the score is the item's mean over
/// replicates, so the item stays the counting unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scored {
    pub score: f64,
    pub weight: f64,
}

fn weighted(items: &[Scored], value: impl Fn(f64) -> f64) -> Option<f64> {
    let w = sum(items.iter().map(|i| i.weight));
    (w > 0.0).then(|| sum(items.iter().map(|i| i.weight * value(i.score))) / w)
}

/// Weighted mean score.
pub fn mean_score(items: &[Scored]) -> Option<f64> {
    weighted(items, |s| s)
}

/// Weighted share of items scoring at least `t`.
pub fn rate_at_least(items: &[Scored], t: f64) -> Option<f64> {
    weighted(items, |s| f64::from(u8::from(s >= t)))
}

/// Unbiased pass@k for one item with `passing` of `replicates` passing:
/// 1 − C(n − c, k) / C(n, k). Needs k ≤ n.
pub fn pass_at_k(passing: u64, replicates: u64, k: u64) -> Option<f64> {
    if k == 0 || k > replicates || passing > replicates {
        return None;
    }
    if replicates - passing < k {
        return Some(1.0);
    }
    // C(n − c, k) / C(n, k) as a product, which stays exact in floating point longer
    // than the binomial coefficients themselves.
    let (n, c) = (replicates as f64, passing as f64);
    let fail_all = (0..k)
        .map(|i| (n - c - i as f64) / (n - i as f64))
        .product::<f64>();
    Some(1.0 - fail_all)
}

/// The mean of per-item pass@k values.
pub fn mean_pass_at_k(items: &[(u64, u64)], k: u64) -> Option<f64> {
    let values: Vec<f64> = items
        .iter()
        .map(|&(c, n)| pass_at_k(c, n, k))
        .collect::<Option<_>>()?;
    (!values.is_empty()).then(|| sum(values.iter().copied()) / values.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_cases() {
        assert_eq!(mean_score(&[]), None);
        assert_eq!(pass_at_k(2, 3, 5), None);
        assert_eq!(pass_at_k(0, 3, 0), None);
        assert_eq!(mean_pass_at_k(&[(1, 2), (2, 2)], 1), Some(0.75));
    }
}
