//! Paired comparison of two arms on the same items, and the verdict rule (metrics
//! spec §7, §7.1).
//!
//! Arm A is the baseline and B the candidate. Differences are B − A in the direction
//! where positive is better.

use serde::Serialize;
use statrs::distribution::{Binomial, Discrete, DiscreteCDF};

use crate::numeric::{norm_cdf, sum};

/// Discordant counts of a paired binary outcome (correct, or positive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Discordant {
    /// A right, B wrong.
    pub only_a: u64,
    /// A wrong, B right.
    pub only_b: u64,
    /// Paired items.
    pub n: u64,
}

impl Discordant {
    /// p_B − p_A.
    pub fn difference(&self) -> Option<f64> {
        (self.n > 0).then(|| (self.only_b as f64 - self.only_a as f64) / self.n as f64)
    }
}

/// Tango's score interval (Tango 1998) for p_B − p_A at the given z, without
/// replicates or clusters (those use the paired bootstrap).
pub fn tango(d: Discordant, z: f64) -> Option<(f64, f64)> {
    let n = d.n as f64;
    if d.n == 0 || d.only_a + d.only_b > d.n {
        return None;
    }
    // In Tango's notation b is positive only in the first arm (B), c only in A.
    let (b, c) = (d.only_b as f64, d.only_a as f64);
    let stat = |delta: f64| {
        // Constrained MLE of the (A only) cell probability given p_B − p_A = delta:
        // the positive root of 2n q² − (b + c + (b − c − 2n)δ) q − c δ(1 − δ) = 0.
        let w = b + c + (b - c - 2.0 * n) * delta;
        let q = (w
            + (w * w + 8.0 * n * c * delta * (1.0 - delta))
                .max(0.0)
                .sqrt())
            / (4.0 * n);
        let var = n * (2.0 * q + delta * (1.0 - delta));
        if var <= 0.0 {
            return if (b - c - n * delta).abs() < f64::EPSILON {
                0.0
            } else {
                (b - c - n * delta).signum() * f64::INFINITY
            };
        }
        (b - c - n * delta) / var.sqrt()
    };
    let point = (b - c) / n;
    // The statistic decreases in delta; find where it crosses +z and −z.
    let root = |target: f64, mut lo: f64, mut hi: f64| {
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if mid <= lo || mid >= hi {
                break;
            }
            if stat(mid) > target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };
    let eps = 1e-12;
    let lower = if stat(-1.0 + eps) <= z {
        -1.0
    } else {
        root(z, -1.0 + eps, point)
    };
    let upper = if stat(1.0 - eps) >= -z {
        1.0
    } else {
        root(-z, point, 1.0 - eps)
    };
    Some((lower, upper))
}

/// The two-sided p-value of Tango's score test of p_B − p_A = 0, the test the
/// interval of [`tango`] inverts: 2(1 − Φ(|b − c| / √(b + c))), 1 without discordant
/// pairs.
pub fn tango_p(d: Discordant) -> f64 {
    let discordant = (d.only_a + d.only_b) as f64;
    if discordant == 0.0 {
        return 1.0;
    }
    let z = (d.only_b as f64 - d.only_a as f64).abs() / discordant.sqrt();
    (2.0 * norm_cdf(-z)).min(1.0)
}

/// McNemar's exact test on the discordant counts: the two-sided exact binomial
/// p-value and the mid-p value.
pub fn mcnemar(only_a: u64, only_b: u64) -> (f64, f64) {
    let n = only_a + only_b;
    if n == 0 {
        return (1.0, 1.0);
    }
    let bin = Binomial::new(0.5, n).expect("valid binomial");
    let m = only_a.min(only_b);
    let exact = (2.0 * bin.cdf(m)).min(1.0);
    let mid = (2.0 * (bin.cdf(m) - 0.5 * bin.pmf(m))).min(1.0);
    (exact, mid)
}

/// Movement of items between the arms (metrics spec §7). Weighted counts, so a soft
/// reference contributes s and 1 − s.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Transition {
    /// Weighted count of items that became positive in arm B and not in arm A, among
    /// reference positives.
    pub new_hits: f64,
    /// Weighted count of reference positives judged positive in arm A and not in arm B.
    pub lost_hits: f64,
    /// Weighted count of reference negatives judged positive in arm B and not in arm A.
    pub new_false_alarms: f64,
    /// Weighted count of reference negatives judged positive in arm A and not in arm B.
    pub cleared_false_alarms: f64,
}

impl Transition {
    /// Add one item: the weight it is positive in the reference, and whether each arm
    /// judged it positive.
    pub fn add(&mut self, positive_weight: f64, a: bool, b: bool) {
        let negative_weight = 1.0 - positive_weight;
        match (a, b) {
            (false, true) => {
                self.new_hits += positive_weight;
                self.new_false_alarms += negative_weight;
            }
            (true, false) => {
                self.lost_hits += positive_weight;
                self.cleared_false_alarms += negative_weight;
            }
            _ => {}
        }
    }
}

/// The verdict of a difference whose positive direction is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The interval is above 0 and the difference reaches the minimum effect.
    Improved,
    /// The difference is smaller than the minimum effect.
    Negligible,
    /// The interval is below 0.
    Worsened,
    /// The interval includes 0 and the difference is not negligible.
    Indistinguishable,
    /// No interval could be formed.
    InsufficientData,
}

/// The verdict rule of metrics spec §7.1, applied in order: inside ±m is
/// `negligible`; an interval below 0 is `worsened`; above 0 is `improved` when the
/// point difference reaches m and `negligible` otherwise; anything else is
/// `indistinguishable`. Without an interval the data are insufficient.
pub fn verdict(delta: f64, interval: Option<(f64, f64)>, min_effect: f64) -> Verdict {
    let Some((lo, hi)) = interval else {
        return Verdict::InsufficientData;
    };
    let m = min_effect;
    if m > 0.0 && -m < lo && hi < m {
        Verdict::Negligible
    } else if hi < 0.0 {
        Verdict::Worsened
    } else if lo > 0.0 && delta >= m {
        Verdict::Improved
    } else if lo > 0.0 {
        Verdict::Negligible
    } else {
        Verdict::Indistinguishable
    }
}

/// Holm's step-down adjustment of p-values, returned in the input order. `None` if
/// any p-value is NaN or outside [0, 1], so an uncomputed test never reads as 1.
pub fn holm(p: &[f64]) -> Option<Vec<f64>> {
    if p.iter().any(|x| !(0.0..=1.0).contains(x)) {
        return None;
    }
    let m = p.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| p[a].total_cmp(&p[b]));
    let mut adjusted = vec![0.0; m];
    let mut running: f64 = 0.0;
    for (rank, &i) in order.iter().enumerate() {
        running = running.max(((m - rank) as f64 * p[i]).min(1.0));
        adjusted[i] = running;
    }
    Some(adjusted)
}

/// One resampling unit of a paired comparison: the sums of an outcome in each arm and
/// the number of paired observations in the unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairedUnit {
    /// Sum of the outcome in arm A.
    pub a: f64,
    /// Sum of the outcome in arm B.
    pub b: f64,
    /// Number of paired observations in the unit.
    pub n: f64,
}

/// The difference of means B − A over the drawn units (both arms share the draw).
pub fn paired_mean_difference(units: &[PairedUnit], drawn: &[usize]) -> Option<f64> {
    let n = sum(drawn.iter().map(|&i| units[i].n));
    (n > 0.0).then(|| {
        (sum(drawn.iter().map(|&i| units[i].b)) - sum(drawn.iter().map(|&i| units[i].a))) / n
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_rules_in_order() {
        use Verdict::*;
        // (delta, interval, m, expected)
        let table = [
            (0.01, Some((-0.01, 0.015)), 0.02, Negligible),
            (-0.05, Some((-0.08, -0.02)), 0.02, Worsened),
            (0.05, Some((0.02, 0.08)), 0.02, Improved),
            (0.01, Some((0.005, 0.03)), 0.02, Negligible),
            (0.01, Some((-0.01, 0.03)), 0.02, Indistinguishable),
            (0.01, Some((0.005, 0.03)), 0.0, Improved),
            (0.0, Some((-0.001, 0.001)), 0.0, Indistinguishable),
            (0.0, None, 0.02, InsufficientData),
            // Inside ±m wins over "worsened": the equivalence form comes first.
            (-0.005, Some((-0.01, -0.001)), 0.02, Negligible),
        ];
        for (d, i, m, want) in table {
            assert_eq!(verdict(d, i, m), want, "{d} {i:?} {m}");
        }
    }

    #[test]
    fn holm_adjusts_in_order() {
        assert_eq!(holm(&[0.01, 0.04, 0.03]), Some(vec![0.03, 0.06, 0.06]));
        assert_eq!(holm(&[]), Some(Vec::new()));
        assert_eq!(holm(&[0.01, f64::NAN]), None);
        assert_eq!(holm(&[1.5]), None);
    }

    #[test]
    fn tango_rejects_inconsistent_counts() {
        assert_eq!(
            tango(
                Discordant {
                    only_a: 30,
                    only_b: 30,
                    n: 40
                },
                1.96
            ),
            None
        );
    }

    #[test]
    fn transitions_weight_a_soft_reference() {
        let mut t = Transition::default();
        t.add(1.0, false, true);
        t.add(0.25, true, false);
        t.add(0.0, false, true);
        assert_eq!(t.new_hits, 1.0);
        assert_eq!(t.new_false_alarms, 1.0);
        assert_eq!(t.lost_hits, 0.25);
        assert_eq!(t.cleared_false_alarms, 0.75);
    }

    #[test]
    fn paired_bootstrap_difference() {
        let units = [
            PairedUnit {
                a: 1.0,
                b: 2.0,
                n: 2.0,
            },
            PairedUnit {
                a: 0.0,
                b: 1.0,
                n: 1.0,
            },
        ];
        assert_eq!(paired_mean_difference(&units, &[0, 1]), Some(2.0 / 3.0));
        assert_eq!(paired_mean_difference(&units, &[]), None);
    }
}
