//! Expected cost and decision thresholds (metrics spec §4).
//!
//! Levels count from the weakest (0). The cost matrix has a row per reference label
//! (negative, positive) and a column per judgment level; `prevalence` is the
//! production prevalence π, not the test set's share. Counts may be weighted, so a soft
//! reference counts item i as s_i positive and 1 − s_i negative.

use serde::Serialize;

use crate::config::Cost;
use crate::numeric::sum;

/// Weights and weighted costs as a negative and as a positive (index 0 and 1), summed
/// over items.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sums {
    /// Weight as a negative, as a positive.
    pub weight: [f64; 2],
    /// Weighted mean cost of the judged levels in the negative and the positive row.
    pub cost: [f64; 2],
}

impl Sums {
    /// Add another item's or cluster's sums.
    pub fn add(&mut self, other: &Sums) {
        for c in 0..2 {
            self.weight[c] += other.weight[c];
            self.cost[c] += other.cost[c];
        }
    }

    /// The weight in both classes.
    pub fn total(&self) -> f64 {
        self.weight[0] + self.weight[1]
    }
}

/// One item's part of expected cost (D-041).
#[derive(Debug, Clone, PartialEq)]
pub struct CostPart {
    /// Cluster the item belongs to.
    pub cluster: String,
    /// The item's value of the `[cost] by` stratum.
    pub stratum: Option<String>,
    /// The item's sums.
    pub sums: Sums,
}

/// How strata are combined into one expected cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mix {
    /// Each stratum's configured production share.
    Configured,
    /// Each stratum's weighted item count, because a stratum present has no share.
    Observed,
}

/// The strata of the parts, in order.
pub fn strata_of<'a>(parts: impl IntoIterator<Item = &'a CostPart>) -> Vec<Option<String>> {
    let set: std::collections::BTreeSet<&Option<String>> =
        parts.into_iter().map(|p| &p.stratum).collect();
    set.into_iter().cloned().collect()
}

/// The parts' sums per stratum of `strata`.
pub fn sums<'a>(
    strata: &[Option<String>],
    parts: impl IntoIterator<Item = &'a CostPart>,
) -> Vec<Sums> {
    let mut out = vec![Sums::default(); strata.len()];
    for p in parts {
        if let Ok(g) = strata.binary_search(&p.stratum) {
            out[g].add(&p.sums);
        }
    }
    out
}

/// Configured when every stratum present has a production share.
pub fn mix(strata: &[Option<String>], cost: &Cost) -> Mix {
    if strata.iter().all(|g| cost.mix_of(g.as_deref()).is_some()) {
        Mix::Configured
    } else {
        Mix::Observed
    }
}

/// How `[cost]` met the data, as (code, message) warnings: strata combined by their
/// counts, and configured strata that no item has (a likely misspelt value).
pub fn notes(cost: &Cost, present: &[Option<String>], mix: Mix) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if cost.by.is_some() && mix == Mix::Observed {
        out.push((
            "stratum_mix_observed",
            "expected cost combines strata by their item counts, not production shares; give every stratum a mix under [cost.strata]".to_owned(),
        ));
    }
    let unmatched: Vec<&str> = cost
        .strata
        .keys()
        .filter(|k| !present.iter().any(|p| p.as_deref() == Some(k.as_str())))
        .map(String::as_str)
        .collect();
    if !unmatched.is_empty() {
        out.push((
            "cost_stratum_unmatched",
            format!(
                "no compared item has these values of the [cost] by stratum, so their settings are unused: {}",
                unmatched.join(", ")
            ),
        ));
    }
    out
}

/// One stratum's EC from its sums; `None` without positives or negatives.
pub fn stratum_value(t: &Sums, prevalence: f64) -> Option<f64> {
    let [wn, wp] = t.weight;
    (wn > 0.0 && wp > 0.0)
        .then(|| prevalence * t.cost[1] / wp + (1.0 - prevalence) * t.cost[0] / wn)
}

/// EC over strata: Σ m_g EC_g / Σ m_g over the strata with items. `None` when a stratum
/// with items has no EC.
pub fn combine(strata: &[Option<String>], sums: &[Sums], cost: &Cost, mix: Mix) -> Option<f64> {
    let (mut num, mut den) = (0.0, 0.0);
    for (g, t) in strata.iter().zip(sums) {
        if t.total() <= 0.0 {
            continue;
        }
        let m = match mix {
            Mix::Configured => cost.mix_of(g.as_deref())?,
            Mix::Observed => t.total(),
        };
        num += m * stratum_value(t, cost.prevalence_of(g.as_deref()))?;
        den += m;
    }
    (den > 0.0).then(|| num / den)
}

fn shares(counts: &[f64]) -> Option<Vec<f64>> {
    let total = sum(counts.iter().copied());
    (total > 0.0).then(|| counts.iter().map(|c| c / total).collect())
}

/// EC = π Σ_j P(j|pos) C\[1\]\[j\] + (1 − π) Σ_j P(j|neg) C\[0\]\[j\].
pub fn expected_cost(
    pos_by_level: &[f64],
    neg_by_level: &[f64],
    matrix: &[Vec<f64>],
    prevalence: f64,
) -> Option<f64> {
    let (p, n) = (shares(pos_by_level)?, shares(neg_by_level)?);
    if matrix.len() != 2 || matrix.iter().any(|row| row.len() != p.len()) || n.len() != p.len() {
        return None;
    }
    let side = |shares: &[f64], row: &[f64]| sum(shares.iter().zip(row).map(|(s, c)| s * c));
    Some(prevalence * side(&p, &matrix[1]) + (1.0 - prevalence) * side(&n, &matrix[0]))
}

/// The cost of binary decisions with hit rate H and false-alarm rate F under a 2×2
/// matrix: π((1−H)C10 + H·C11) + (1−π)(F·C01 + (1−F)C00).
pub fn binary_cost(h: f64, f: f64, matrix: &[[f64; 2]; 2], prevalence: f64) -> f64 {
    prevalence * ((1.0 - h) * matrix[1][0] + h * matrix[1][1])
        + (1.0 - prevalence) * (f * matrix[0][1] + (1.0 - f) * matrix[0][0])
}

/// The expected cost of calling levels ≥ t positive, for t = 1 … k − 1.
pub fn threshold_costs(
    pos_by_level: &[f64],
    neg_by_level: &[f64],
    matrix: &[[f64; 2]; 2],
    prevalence: f64,
) -> Option<Vec<(usize, f64)>> {
    let (p, n) = (shares(pos_by_level)?, shares(neg_by_level)?);
    Some(
        (1..p.len().min(n.len()))
            .map(|t| {
                let h = sum(p[t..].iter().copied());
                let f = sum(n[t..].iter().copied());
                (t, binary_cost(h, f, matrix, prevalence))
            })
            .collect(),
    )
}

/// The threshold with the least expected cost (the lower one on a tie).
///
/// Costs that are equal in exact arithmetic differ in their last digits, so a cost within
/// 1e-12 of the largest matrix entry counts as the least.
pub fn optimal_threshold(
    pos_by_level: &[f64],
    neg_by_level: &[f64],
    matrix: &[[f64; 2]; 2],
    prevalence: f64,
) -> Option<(usize, f64)> {
    let costs = threshold_costs(pos_by_level, neg_by_level, matrix, prevalence)?;
    let least = costs.iter().map(|c| c.1).min_by(f64::total_cmp)?;
    let scale = matrix
        .iter()
        .flatten()
        .fold(0.0, |m: f64, c| m.max(c.abs()));
    costs.into_iter().find(|c| c.1 <= least + 1e-12 * scale)
}

/// The matrix with C_FP = 1, C_FN = r and no cost for correct decisions.
pub fn ratio_matrix(r: f64) -> [[f64; 2]; 2] {
    [[0.0, 1.0], [r, 0.0]]
}

/// The optimal threshold at both ends of the cost-ratio range r = C_FN / C_FP.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct ThresholdSensitivity {
    /// Optimal threshold level at the low end of the cost-ratio range.
    pub at_low: usize,
    /// Optimal threshold level at the high end of the cost-ratio range.
    pub at_high: usize,
    /// Whether the optimal threshold differs between the two ends.
    pub changes: bool,
}

/// The optimal threshold at both ends of the range `[lo, hi]` of r = C_FN / C_FP; `None` if
/// undefined.
pub fn threshold_sensitivity(
    pos_by_level: &[f64],
    neg_by_level: &[f64],
    prevalence: f64,
    [lo, hi]: [f64; 2],
) -> Option<ThresholdSensitivity> {
    let at = |r| {
        optimal_threshold(pos_by_level, neg_by_level, &ratio_matrix(r), prevalence).map(|(t, _)| t)
    };
    let (at_low, at_high) = (at(lo)?, at(hi)?);
    Some(ThresholdSensitivity {
        at_low,
        at_high,
        changes: at_low != at_high,
    })
}

/// EC(a) − EC(b) for two arms' (H, F) at both ends of the cost-ratio range.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct DifferenceSensitivity {
    /// EC(a) − EC(b) at the low end of the cost-ratio range.
    pub at_low: f64,
    /// EC(a) − EC(b) at the high end of the cost-ratio range.
    pub at_high: f64,
    /// Whether the difference changes sign between the two ends.
    pub sign_changes: bool,
}

/// The expected-cost difference of two arms at both ends of the range `[lo, hi]` of r =
/// C_FN / C_FP.
pub fn difference_sensitivity(
    a: (f64, f64),
    b: (f64, f64),
    prevalence: f64,
    [lo, hi]: [f64; 2],
) -> DifferenceSensitivity {
    let diff = |r| {
        let m = ratio_matrix(r);
        binary_cost(a.0, a.1, &m, prevalence) - binary_cost(b.0, b.1, &m, prevalence)
    };
    let (at_low, at_high) = (diff(lo), diff(hi));
    // Differences equal in exact arithmetic differ from 0 in their last digits.
    let tie = |d: f64| d.abs() <= 1e-12 * lo.abs().max(hi.abs()).max(1.0);
    DifferenceSensitivity {
        at_low,
        at_high,
        // A tie at either end is no change of sign.
        sign_changes: !tie(at_low) && !tie(at_high) && (at_low < 0.0) != (at_high < 0.0),
    }
}

/// The posterior probability above which calling positive costs less:
/// p* = C_FP / (C_FP + C_FN).
pub fn posterior_threshold(c_fp: f64, c_fn: f64) -> f64 {
    c_fp / (c_fp + c_fn)
}

/// The same decision as a likelihood ratio: LR* = (C_FP / C_FN)(1 − π) / π.
pub fn lr_threshold(c_fp: f64, c_fn: f64, prevalence: f64) -> f64 {
    (c_fp / c_fn) * (1.0 - prevalence) / prevalence
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StratumCost;

    fn cost(strata: &[(&str, Option<f64>, Option<f64>)]) -> Cost {
        Cost {
            matrix: vec![vec![0.0, 1.0], vec![4.0, 0.0]],
            prevalence: 0.2,
            ratio_range: None,
            by: Some("s".into()),
            strata: strata
                .iter()
                .map(|(k, prevalence, mix)| {
                    (
                        (*k).to_owned(),
                        StratumCost {
                            prevalence: *prevalence,
                            matrix: None,
                            mix: *mix,
                        },
                    )
                })
                .collect(),
        }
    }

    fn sums(weight: [f64; 2], cost: [f64; 2]) -> Sums {
        Sums { weight, cost }
    }

    #[test]
    fn a_stratum_needs_both_classes() {
        assert_eq!(stratum_value(&sums([0.0, 2.0], [0.0, 1.0]), 0.5), None);
        // π·(1/2) + (1 − π)·(3/4) at π = 0.2.
        let v = stratum_value(&sums([4.0, 2.0], [3.0, 1.0]), 0.2).unwrap();
        assert!((v - (0.2 * 0.5 + 0.8 * 0.75)).abs() < 1e-12);
    }

    #[test]
    fn strata_combine_by_configured_shares_or_by_counts() {
        let strata = vec![Some("a".to_owned()), Some("b".to_owned())];
        let s = [sums([2.0, 2.0], [2.0, 2.0]), sums([6.0, 2.0], [0.0, 0.0])];
        // EC_a = 0.3·1 + 0.7·1 = 1; EC_b = 0.
        let c = cost(&[("a", Some(0.3), Some(0.25)), ("b", None, Some(0.75))]);
        assert_eq!(mix(&strata, &c), Mix::Configured);
        assert!((combine(&strata, &s, &c, Mix::Configured).unwrap() - 0.25).abs() < 1e-12);
        let c = cost(&[("a", Some(0.3), Some(0.25))]);
        assert_eq!(mix(&strata, &c), Mix::Observed);
        // Weighted by counts 4 and 8.
        assert!((combine(&strata, &s, &c, Mix::Observed).unwrap() - 4.0 / 12.0).abs() < 1e-12);
        // A stratum without negatives makes the whole value undefined.
        let s = [sums([0.0, 2.0], [0.0, 2.0]), s[1]];
        assert_eq!(combine(&strata, &s, &c, Mix::Observed), None);
        // An empty stratum is skipped.
        let s = [Sums::default(), sums([6.0, 2.0], [0.0, 0.0])];
        assert_eq!(combine(&strata, &s, &c, Mix::Observed), Some(0.0));
    }

    #[test]
    fn stratum_settings_fall_back_to_the_defaults() {
        let c = cost(&[("a", Some(0.3), None)]);
        assert_eq!(c.prevalence_of(Some("a")), 0.3);
        assert_eq!(c.prevalence_of(Some("b")), 0.2);
        assert_eq!(c.prevalence_of(None), 0.2);
        assert_eq!(c.matrix_of(Some("a")), c.matrix.as_slice());
        assert_eq!(c.mix_of(Some("a")), None);
    }

    #[test]
    fn notes_name_unmatched_strata_and_an_observed_mix() {
        let c = cost(&[("S1", None, None)]);
        let found = notes(&c, &[Some("S-1".to_owned())], Mix::Observed);
        let codes: Vec<&str> = found.iter().map(|n| n.0).collect();
        assert_eq!(codes, ["stratum_mix_observed", "cost_stratum_unmatched"]);
        assert!(found[1].1.ends_with("S1"), "{}", found[1].1);
        let c = cost(&[("S-1", None, Some(1.0))]);
        assert!(notes(&c, &[Some("S-1".to_owned())], Mix::Configured).is_empty());
    }
}
