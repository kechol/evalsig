//! ROC, AUC, the DeLong interval and comparison, z-ROC points and calibration
//! (metrics spec §3).
//!
//! Scores are oriented so that larger means more positive (ordinal levels count up
//! from the weakest). Ties count one half.

use statrs::distribution::{ContinuousCDF, Normal};

use crate::bootstrap::{self, MIN_UNITS};
use crate::numeric::{Rng, Running, norm_ppf, sum, variance};

/// ψ(x, y): 1 if the positive scores higher, ½ on a tie, 0 below.
pub fn psi(x: f64, y: f64) -> f64 {
    if x > y {
        1.0
    } else if x == y {
        0.5
    } else {
        0.0
    }
}

/// A score with the weight it carries (1/K_i for one of an item's K_i replicates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weighted {
    /// The score.
    pub score: f64,
    /// The weight of the score.
    pub weight: f64,
}

/// The AUC of weighted positive and negative scores: Σ w_i w_j ψ / Σ w_i w_j, the soft
/// AUC of items whose shares are 1 and 0.
pub fn auc(pos: &[Weighted], neg: &[Weighted]) -> Option<f64> {
    let item = |w: &Weighted, share: f64| SoftItem {
        score: w.score,
        share,
        weight: w.weight,
    };
    let items: Vec<SoftItem> = pos
        .iter()
        .map(|w| item(w, 1.0))
        .chain(neg.iter().map(|w| item(w, 0.0)))
        .collect();
    auc_soft(&items)
}

/// The empirical ROC as (false-alarm rate, hit rate) points: one threshold at each
/// distinct score from the strongest, with (0, 0) and (1, 1) at the ends.
pub fn roc_points(pos: &[Weighted], neg: &[Weighted]) -> Vec<(f64, f64)> {
    let total = |xs: &[Weighted]| sum(xs.iter().map(|x| x.weight));
    let (tp, tn) = (total(pos), total(neg));
    if tp <= 0.0 || tn <= 0.0 || pos.iter().chain(neg).any(|x| x.score.is_nan()) {
        return Vec::new();
    }
    // One sweep from the strongest score, accumulating each score's weights.
    let mut scored: Vec<(f64, f64, f64)> = pos
        .iter()
        .map(|x| (x.score, x.weight, 0.0))
        .chain(neg.iter().map(|x| (x.score, 0.0, x.weight)))
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out = vec![(0.0, 0.0)];
    let (mut hits, mut alarms, mut start) = (Running::default(), Running::default(), 0);
    while start < scored.len() {
        let end = start
            + scored[start..]
                .iter()
                .take_while(|x| x.0 == scored[start].0)
                .count();
        for x in &scored[start..end] {
            hits.add(x.1);
            alarms.add(x.2);
        }
        out.push((alarms.total() / tn, hits.total() / tp));
        start = end;
    }
    if out.last() != Some(&(1.0, 1.0)) {
        out.push((1.0, 1.0));
    }
    out
}

/// An item against a soft reference: its score, the share s of raters who call it
/// positive, and its weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftItem {
    /// The item's score.
    pub score: f64,
    /// Share s of raters who call the item positive, in [0, 1].
    pub share: f64,
    /// The item's weight.
    pub weight: f64,
}

/// AUC against a soft reference: Σ s_i(1−s_j)ψ(i,j) / Σ s_i(1−s_j) over all pairs,
/// i = j included with ψ = ½. Sorted by score, each item's positive mass meets the
/// negative mass of every lower score, and half of its own score's, in O(n log n).
pub fn auc_soft(items: &[SoftItem]) -> Option<f64> {
    if items.iter().any(|x| x.score.is_nan()) {
        return None;
    }
    let mut sorted: Vec<&SoftItem> = items.iter().collect();
    sorted.sort_by(|a, b| a.score.total_cmp(&b.score));
    let pos = |x: &SoftItem| x.weight * x.share;
    let neg = |x: &SoftItem| x.weight * (1.0 - x.share);
    let den = sum(items.iter().map(pos)) * sum(items.iter().map(neg));
    if den <= 0.0 {
        return None;
    }
    let (mut below, mut num, mut start) = (0.0, Vec::new(), 0);
    while start < sorted.len() {
        let end = start
            + sorted[start..]
                .iter()
                .take_while(|x| x.score == sorted[start].score)
                .count();
        let group = &sorted[start..end];
        let (p, n) = (
            sum(group.iter().map(|x| pos(x))),
            sum(group.iter().map(|x| neg(x))),
        );
        num.push(p * (below + 0.5 * n));
        below += n;
        start = end;
    }
    Some(sum(num) / den)
}

/// A percentile bootstrap interval of the soft AUC, resampling clusters (`cluster[i]`
/// numbers item i's cluster from 0). `None` with fewer than `MIN_UNITS` clusters.
pub fn auc_soft_interval(
    items: &[SoftItem],
    cluster: &[usize],
    b: u32,
    seed: u64,
    level: f64,
) -> Option<(f64, f64)> {
    let n = cluster.iter().max().map_or(0, |m| m + 1);
    if n < MIN_UNITS {
        return None;
    }
    let mut members = vec![Vec::new(); n];
    for (i, c) in cluster.iter().enumerate() {
        members[*c].push(items[i]);
    }
    let mut rng = Rng::new(seed);
    let (reps, _) = bootstrap::replicates(n, None, b, &mut rng, |drawn| {
        let sample: Vec<SoftItem> = drawn
            .iter()
            .flat_map(|&c| members[c].iter().copied())
            .collect();
        auc_soft(&sample)
    });
    bootstrap::percentile_interval(reps, level)
}

/// The DeLong estimate for one AUC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeLong {
    /// The area under the ROC curve (Mann-Whitney statistic).
    pub auc: f64,
    /// DeLong variance of the AUC.
    pub variance: f64,
    /// On the logit scale and transformed back; `None` when the AUC is 0 or 1, where
    /// the variance degenerates and a bootstrap is used instead.
    pub interval: Option<(f64, f64)>,
}

fn covariance(xs: &[f64], ys: &[f64]) -> f64 {
    let (mx, my) = (
        sum(xs.iter().copied()) / xs.len() as f64,
        sum(ys.iter().copied()) / ys.len() as f64,
    );
    sum(xs.iter().zip(ys).map(|(x, y)| (x - mx) * (y - my))) / (xs.len() as f64 - 1.0)
}

/// Structural components V10 (per positive) and V01 (per negative).
fn components(pos: &[f64], neg: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let sorted = |xs: &[f64]| {
        let mut v = xs.to_vec();
        v.sort_by(f64::total_cmp);
        v
    };
    let (sp, sn) = (sorted(pos), sorted(neg));
    // ψ summed over the other class: those strictly below count 1, ties ½.
    let below_and_tied = |others: &[f64], x: f64| {
        let below = others.partition_point(|y| *y < x);
        let at_most = others.partition_point(|y| *y <= x);
        (below as f64, (at_most - below) as f64)
    };
    let v10 = pos
        .iter()
        .map(|&x| {
            let (b, t) = below_and_tied(&sn, x);
            (b + 0.5 * t) / neg.len() as f64
        })
        .collect();
    let v01 = neg
        .iter()
        .map(|&y| {
            let (b, t) = below_and_tied(&sp, y);
            let above = pos.len() as f64 - b - t;
            (above + 0.5 * t) / pos.len() as f64
        })
        .collect();
    (v10, v01)
}

/// DeLong's variance of an AUC from one score per item, and its interval at `level`.
/// Needs at least two positives and two negatives.
pub fn delong(pos: &[f64], neg: &[f64], level: f64) -> Option<DeLong> {
    if pos.len() < 2 || neg.len() < 2 {
        return None;
    }
    let (v10, v01) = components(pos, neg);
    let auc = sum(v10.iter().copied()) / v10.len() as f64;
    let variance = variance(&v10, 1) / pos.len() as f64 + variance(&v01, 1) / neg.len() as f64;
    Some(DeLong {
        auc,
        variance,
        interval: logit_interval(auc, variance, level),
    })
}

/// The logit-scale interval of an AUC with this variance; `None` at 0 or 1.
fn logit_interval(auc: f64, variance: f64, level: f64) -> Option<(f64, f64)> {
    (auc > 0.0 && auc < 1.0 && variance > 0.0).then(|| {
        let se = variance.sqrt() / (auc * (1.0 - auc));
        let logit = (auc / (1.0 - auc)).ln();
        let z = norm_ppf(0.5 + level / 2.0);
        let back = |x: f64| 1.0 / (1.0 + (-x).exp());
        (back(logit - z * se), back(logit + z * se))
    })
}

/// The AUC of items in clusters (Obuchowski 1997): the DeLong components are summed
/// per cluster, V10_k and V01_k, and with K clusters, M positives and N negatives
/// S10 = K/((K−1)M) Σ (V10_k − m_k A)², S01 = K/((K−1)N) Σ (V01_k − n_k A)²,
/// S11 = K/(K−1) Σ (V10_k − m_k A)(V01_k − n_k A), Var(A) = S10/M + S01/N + 2 S11/(MN).
/// `pos` and `neg` are (score, cluster index) pairs.
pub fn delong_clustered(pos: &[(f64, usize)], neg: &[(f64, usize)], level: f64) -> Option<DeLong> {
    let k = pos.iter().chain(neg).map(|(_, c)| c + 1).max().unwrap_or(0);
    if pos.len() < 2 || neg.len() < 2 || k < 2 {
        return None;
    }
    let (ps, ns): (Vec<f64>, Vec<f64>) = (
        pos.iter().map(|p| p.0).collect(),
        neg.iter().map(|n| n.0).collect(),
    );
    let (v10, v01) = components(&ps, &ns);
    let (m, n) = (pos.len() as f64, neg.len() as f64);
    let auc = sum(v10.iter().copied()) / m;
    let (mut c10, mut c01, mut mk, mut nk) =
        (vec![0.0; k], vec![0.0; k], vec![0.0; k], vec![0.0; k]);
    for (v, (_, c)) in v10.iter().zip(pos) {
        c10[*c] += v;
        mk[*c] += 1.0;
    }
    for (v, (_, c)) in v01.iter().zip(neg) {
        c01[*c] += v;
        nk[*c] += 1.0;
    }
    let kf = k as f64;
    let d10: Vec<f64> = (0..k).map(|c| c10[c] - mk[c] * auc).collect();
    let d01: Vec<f64> = (0..k).map(|c| c01[c] - nk[c] * auc).collect();
    let s10 = kf / ((kf - 1.0) * m) * sum(d10.iter().map(|d| d * d));
    let s01 = kf / ((kf - 1.0) * n) * sum(d01.iter().map(|d| d * d));
    let s11 = kf / (kf - 1.0) * sum(d10.iter().zip(&d01).map(|(a, b)| a * b));
    let variance = s10 / m + s01 / n + 2.0 * s11 / (m * n);
    Some(DeLong {
        auc,
        variance,
        interval: logit_interval(auc, variance, level),
    })
}

/// The paired DeLong comparison of two scores on the same items.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairedAuc {
    /// AUC of score a.
    pub auc_a: f64,
    /// AUC of score b.
    pub auc_b: f64,
    /// AUC(b) − AUC(a).
    pub difference: f64,
    /// DeLong variance of the AUC difference.
    pub variance: f64,
    /// z statistic of the difference.
    pub z: f64,
    /// Two-sided p-value of the z statistic.
    pub p_two_sided: f64,
    /// Interval of the AUC difference.
    pub interval: (f64, f64),
}

/// Compare the AUCs of scores `a` and `b` on the same items with reference `labels`.
pub fn delong_paired(labels: &[bool], a: &[f64], b: &[f64], level: f64) -> Option<PairedAuc> {
    let split = |s: &[f64]| -> (Vec<f64>, Vec<f64>) {
        let pos = s
            .iter()
            .zip(labels)
            .filter(|(_, l)| **l)
            .map(|(x, _)| *x)
            .collect();
        let neg = s
            .iter()
            .zip(labels)
            .filter(|(_, l)| !**l)
            .map(|(x, _)| *x)
            .collect();
        (pos, neg)
    };
    let ((ap, an), (bp, bn)) = (split(a), split(b));
    if ap.len() < 2 || an.len() < 2 || a.len() != b.len() || a.len() != labels.len() {
        return None;
    }
    let ((a10, a01), (b10, b01)) = (components(&ap, &an), components(&bp, &bn));
    let (m, n) = (a10.len() as f64, a01.len() as f64);
    let var = |x10: &[f64], x01: &[f64]| variance(x10, 1) / m + variance(x01, 1) / n;
    let cov = covariance(&a10, &b10) / m + covariance(&a01, &b01) / n;
    let variance = var(&a10, &a01) + var(&b10, &b01) - 2.0 * cov;
    let (auc_a, auc_b) = (sum(a10.iter().copied()) / m, sum(b10.iter().copied()) / m);
    let difference = auc_b - auc_a;
    if variance <= 0.0 {
        return None;
    }
    let se = variance.sqrt();
    let z = difference / se;
    let normal = Normal::standard();
    let half = norm_ppf(0.5 + level / 2.0) * se;
    Some(PairedAuc {
        auc_a,
        auc_b,
        difference,
        variance,
        z,
        p_two_sided: 2.0 * normal.sf(z.abs()),
        interval: (difference - half, difference + half),
    })
}

/// z-ROC points (z(F′), z(H′)) at each threshold between levels, with the log-linear
/// correction (k + ½)/(n + 1). Histograms count items per level from the strongest.
pub fn zroc_points(hist_pos: &[u64], hist_neg: &[u64]) -> Vec<(f64, f64)> {
    let (np, nn) = (
        hist_pos.iter().sum::<u64>() as f64,
        hist_neg.iter().sum::<u64>() as f64,
    );
    (1..hist_pos.len().min(hist_neg.len()))
        .map(|i| {
            let kh = hist_pos[..i].iter().sum::<u64>() as f64;
            let kf = hist_neg[..i].iter().sum::<u64>() as f64;
            (
                norm_ppf((kf + 0.5) / (nn + 1.0)),
                norm_ppf((kh + 0.5) / (np + 1.0)),
            )
        })
        .collect()
}

/// One reliability bin: scores in [bin/10, (bin+1)/10), the last including 1.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Bin {
    /// Index of the bin, from 0 to 9.
    pub bin: usize,
    /// Number of items in the bin.
    pub n: usize,
    /// Mean score of the items in the bin.
    pub mean_score: f64,
    /// Share of positives among the items in the bin.
    pub positive_share: f64,
}

/// The Brier score and the 10 equal-width reliability bins that hold scores, for
/// scores in [0, 1] against references y (a soft share or 0/1), with item `weights`.
/// A bin's `n` counts items; its mean score and positive share are weighted.
pub fn calibration(scores: &[f64], y: &[f64], weights: &[f64]) -> Option<(f64, Vec<Bin>)> {
    if scores.is_empty()
        || scores.len() != y.len()
        || scores.len() != weights.len()
        || scores.iter().any(|p| !(0.0..=1.0).contains(p))
    {
        return None;
    }
    let brier = sum(scores
        .iter()
        .zip(y)
        .zip(weights)
        .map(|((p, t), w)| w * (p - t) * (p - t)))
        / sum(weights.iter().copied());
    let bins = (0..10)
        .filter_map(|b| {
            let members: Vec<(f64, f64, f64)> = scores
                .iter()
                .zip(y)
                .zip(weights)
                .filter(|((p, _), _)| ((**p * 10.0) as usize).min(9) == b)
                .map(|((p, t), w)| (*p, *t, *w))
                .collect();
            let total = sum(members.iter().map(|m| m.2));
            (!members.is_empty()).then(|| Bin {
                bin: b,
                n: members.len(),
                mean_score: sum(members.iter().map(|m| m.2 * m.0)) / total,
                positive_share: sum(members.iter().map(|m| m.2 * m.1)) / total,
            })
        })
        .collect();
    Some((brier, bins))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn unit(xs: &[f64]) -> Vec<Weighted> {
        xs.iter()
            .map(|&score| Weighted { score, weight: 1.0 })
            .collect()
    }

    proptest! {
        #[test]
        fn auc_is_invariant_under_monotone_transforms(
            pos in prop::collection::vec(-5.0f64..5.0, 1..20),
            neg in prop::collection::vec(-5.0f64..5.0, 1..20),
            scale in 0.1f64..10.0,
            shift in -3.0f64..3.0,
        ) {
            let f = |x: f64| (x * scale + shift).exp();
            let base = auc(&unit(&pos), &unit(&neg)).unwrap();
            let pos2: Vec<f64> = pos.iter().map(|&x| f(x)).collect();
            let neg2: Vec<f64> = neg.iter().map(|&x| f(x)).collect();
            let moved = auc(&unit(&pos2), &unit(&neg2)).unwrap();
            prop_assert!((base - moved).abs() < 1e-12);
        }
    }

    #[test]
    fn a_soft_reference_of_zeros_and_ones_is_the_binary_auc() {
        let items = [(0.9, 1.0), (0.7, 1.0), (0.6, 0.0), (0.2, 0.0), (0.8, 0.0)];
        let soft: Vec<SoftItem> = items
            .iter()
            .map(|&(score, share)| SoftItem {
                score,
                share,
                weight: 1.0,
            })
            .collect();
        let binary = auc(&unit(&[0.9, 0.7]), &unit(&[0.6, 0.2, 0.8])).unwrap();
        assert!((auc_soft(&soft).unwrap() - binary).abs() < 1e-12);
        let clusters: Vec<usize> = (0..5).collect();
        let (lo, hi) = auc_soft_interval(&soft, &clusters, 500, 7, 0.95).unwrap();
        assert!(lo <= binary && binary <= hi);
    }

    #[test]
    fn a_nan_score_has_no_roc_and_no_auc() {
        let nan = unit(&[f64::NAN, 1.0]);
        assert!(roc_points(&nan, &unit(&[0.5])).is_empty());
        let items = [
            SoftItem {
                score: f64::NAN,
                share: 1.0,
                weight: 1.0,
            },
            SoftItem {
                score: 0.5,
                share: 0.0,
                weight: 1.0,
            },
        ];
        assert_eq!(auc_soft(&items), None);
        assert_eq!(
            crate::agreement::spearman(&[f64::NAN, 1.0], &[1.0, 2.0]),
            None
        );
    }

    #[test]
    fn roc_points_run_from_origin_to_corner() {
        let pts = roc_points(&unit(&[3.0, 2.0]), &unit(&[2.0, 1.0]));
        assert_eq!(pts, [(0.0, 0.0), (0.0, 0.5), (0.5, 1.0), (1.0, 1.0)]);
    }
}
