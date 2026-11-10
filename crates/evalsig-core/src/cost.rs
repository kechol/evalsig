//! Expected cost and decision thresholds (metrics spec §4).
//!
//! Levels count from the weakest (0). The cost matrix has a row per reference label
//! (negative, positive) and a column per judgment level; `prevalence` is the
//! production prevalence π, not the test set's share. Counts may be weighted, so a soft
//! reference counts item i as s_i positive and 1 − s_i negative.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bootstrap;
use crate::config::Cost;
use crate::numeric::{Rng, sum};

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

/// Items' cost parts grouped by cluster for the bootstrap, in `A` arms that share items:
/// each cluster keeps the sums of the strata its items fall in, taken once.
pub struct Clustered<const A: usize> {
    /// The strata of the items, in order.
    pub strata: Vec<Option<String>>,
    clusters: Vec<Vec<(usize, [Sums; A])>>,
}

impl<const A: usize> Clustered<A> {
    /// Group `items`, which every arm has, by the first arm's clusters and strata.
    pub fn new(items: &[&str], arms: [&BTreeMap<String, CostPart>; A]) -> Self {
        let first = arms[0];
        let strata: Vec<Option<String>> = items
            .iter()
            .map(|i| first[*i].stratum.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut by_cluster: BTreeMap<&str, Vec<(usize, [Sums; A])>> = BTreeMap::new();
        for i in items {
            let g = strata
                .binary_search(&first[*i].stratum)
                .expect("the stratum of an item");
            let entries = by_cluster.entry(first[*i].cluster.as_str()).or_default();
            let at = match entries.iter().position(|e| e.0 == g) {
                Some(at) => at,
                None => {
                    entries.push((g, [Sums::default(); A]));
                    entries.len() - 1
                }
            };
            for (sums, arm) in entries[at].1.iter_mut().zip(arms) {
                sums.add(&arm[*i].sums);
            }
        }
        Clustered {
            strata,
            clusters: by_cluster.into_values().collect(),
        }
    }

    /// Number of clusters, the bootstrap's resampling units.
    pub fn n_clusters(&self) -> usize {
        self.clusters.len()
    }

    /// Stratum `g`'s interval from its replicates, when it has a value and at least
    /// [`bootstrap::MIN_UNITS`] clusters with items in it.
    pub fn stratum_ci(
        &self,
        g: usize,
        has_value: bool,
        reps: Vec<f64>,
        level: f64,
    ) -> Option<(f64, f64)> {
        let carrying = self
            .clusters
            .iter()
            .filter(|entries| entries.iter().any(|e| e.0 == g))
            .count();
        (has_value && carrying >= bootstrap::MIN_UNITS)
            .then(|| bootstrap::percentile_interval(reps, level))
            .flatten()
    }

    /// Each arm's per-stratum sums over the drawn clusters.
    pub fn sums(&self, drawn: impl Iterator<Item = usize>) -> [Vec<Sums>; A] {
        let mut out: [Vec<Sums>; A] =
            std::array::from_fn(|_| vec![Sums::default(); self.strata.len()]);
        for c in drawn {
            for (g, arms) in &self.clusters[c] {
                for (acc, sums) in out.iter_mut().zip(arms) {
                    acc[*g].add(sums);
                }
            }
        }
        out
    }

    /// Replicates of the overall statistic, then, with `per_stratum`, of each stratum's,
    /// from one set of resamples; empty lists below [`bootstrap::MIN_UNITS`] clusters.
    pub fn replicates(
        &self,
        b: u32,
        seed: u64,
        per_stratum: bool,
        overall: impl Fn(&[Vec<Sums>; A]) -> Option<f64>,
        stratum: impl Fn(&[Vec<Sums>; A], usize) -> Option<f64>,
    ) -> Vec<Vec<f64>> {
        let k = if per_stratum {
            self.strata.len() + 1
        } else {
            1
        };
        if self.n_clusters() < bootstrap::MIN_UNITS {
            return vec![Vec::new(); k];
        }
        let mut rng = Rng::new(seed);
        bootstrap::replicates_many(self.n_clusters(), None, b, &mut rng, k, |drawn| {
            let s = self.sums(drawn.iter().copied());
            std::iter::once(overall(&s))
                .chain((1..k).map(|g| stratum(&s, g - 1)))
                .collect::<Vec<_>>()
        })
    }
}

/// A run's expected cost in `report` (D-041).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CostSummary {
    /// Expected cost over the strata; `None` when a stratum lacks positives or negatives.
    pub value: Option<f64>,
    /// Cluster-bootstrap interval.
    pub ci: Option<(f64, f64)>,
    /// `bootstrap`, `too_few_units` or `none`.
    pub interval: &'static str,
    /// Items with a reference and a judged level.
    pub n_used: usize,
    /// With a `[cost] by` stratum: how strata were combined and one row per stratum.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub by_stratum: Option<StrataCost>,
}

/// Expected cost per stratum of one run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StrataCost {
    /// How strata were combined into the overall value.
    pub mix: Mix,
    /// One row per stratum.
    pub strata: Vec<StratumCost>,
}

impl StrataCost {
    /// The strata present.
    pub fn present(&self) -> Vec<Option<String>> {
        self.strata.iter().map(|s| s.stratum.clone()).collect()
    }
}

/// Expected cost in one stratum of one run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StratumCost {
    /// The stratum's value; `None` for items without one.
    pub stratum: Option<String>,
    /// The stratum's production prevalence.
    pub prevalence: f64,
    /// Weighted number of items in the stratum.
    pub n: f64,
    /// Expected cost in the stratum.
    pub value: Option<f64>,
    /// Cluster-bootstrap interval; none below five clusters with items in the stratum.
    pub ci: Option<(f64, f64)>,
}

/// One run's expected cost overall and per stratum, with cluster-bootstrap intervals.
pub fn summary(
    parts: &BTreeMap<String, CostPart>,
    cost: &Cost,
    b: u32,
    seed: u64,
    level: f64,
) -> CostSummary {
    let items: Vec<&str> = parts.keys().map(String::as_str).collect();
    let groups = Clustered::new(&items, [parts]);
    let strata = &groups.strata;
    let mix = mix(strata, cost);
    let overall = |s: &[Vec<Sums>; 1]| combine(strata, &s[0], cost, mix);
    let one = |s: &[Vec<Sums>; 1], g: usize| {
        stratum_value(&s[0][g], cost.prevalence_of(strata[g].as_deref()))
    };
    let all = groups.sums(0..groups.n_clusters());
    let value = overall(&all);
    let mut reps = groups
        .replicates(b, seed, cost.by.is_some(), overall, one)
        .into_iter();
    let first = reps.next().expect("one list per statistic");
    let (ci, interval) = match value {
        None => (None, "none"),
        Some(_) if groups.n_clusters() < bootstrap::MIN_UNITS => (None, "too_few_units"),
        Some(_) => (bootstrap::percentile_interval(first, level), "bootstrap"),
    };
    CostSummary {
        value,
        ci,
        interval,
        n_used: items.len(),
        by_stratum: cost.by.is_some().then(|| StrataCost {
            mix,
            strata: strata
                .iter()
                .zip(reps)
                .enumerate()
                .map(|(g, (stratum, r))| {
                    let value = one(&all, g);
                    StratumCost {
                        stratum: stratum.clone(),
                        prevalence: cost.prevalence_of(stratum.as_deref()),
                        n: all[0][g].total(),
                        value,
                        ci: groups.stratum_ci(g, value.is_some(), r, level),
                    }
                })
                .collect(),
        }),
    }
}

/// Configured when every stratum present has a production share.
pub fn mix(strata: &[Option<String>], cost: &Cost) -> Mix {
    if strata.iter().all(|g| cost.mix_of(g.as_deref()).is_some()) {
        Mix::Configured
    } else {
        Mix::Observed
    }
}

/// The warning when `[cost]` is set but a run's judgments give no expected cost.
pub const NOT_COMPUTED: (&str, &str) = (
    "expected_cost_not_computed",
    "[cost] is set but expected cost needs every judged value to be a level 0 … k−1 of its matrix, and a reference",
);

/// How `[cost] by` met the data, as (code, message) warnings: strata combined by their
/// counts, and configured strata that no item has (a likely misspelt value).
pub fn notes(cost: &Cost, present: &[Option<String>], mix: Mix) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if mix == Mix::Observed {
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
                "no item has these values of the [cost] by stratum, so their settings are unused: {}",
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
