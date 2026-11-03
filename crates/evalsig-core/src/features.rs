//! Feature likelihood ratios (metrics spec §5): how much more often a feature fires on
//! positives than on negatives, with the Katz log interval and, for many features at
//! once, intervals at the Benjamini-Hochberg (FCR) adjusted level.

use serde::Serialize;
use statrs::distribution::{ContinuousCDF, Normal};

use crate::numeric::norm_ppf;

/// Firing counts of one feature: `a` of `n_pos` positives and `b` of `n_neg`
/// negatives. Counts may be fractional (replicate shares, soft references).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Counts {
    pub a: f64,
    pub n_pos: f64,
    pub b: f64,
    pub n_neg: f64,
}

impl Counts {
    /// The cells used: with a zero `a` or `b`, ½ is added to all four cells
    /// (Haldane-Anscombe), so the denominators gain 1.
    fn cells(self) -> (f64, f64, f64, f64, bool) {
        if self.a > 0.0 && self.b > 0.0 {
            (self.a, self.n_pos, self.b, self.n_neg, false)
        } else {
            (
                self.a + 0.5,
                self.n_pos + 1.0,
                self.b + 0.5,
                self.n_neg + 1.0,
                true,
            )
        }
    }

    pub fn fired(self) -> f64 {
        self.a + self.b
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Ratio {
    pub lr: f64,
    /// The standard error of log LR.
    pub se: f64,
    pub interval: (f64, f64),
    pub corrected: bool,
}

/// LR = (a/n_pos)/(b/n_neg) with its Katz log interval at `level`.
pub fn likelihood_ratio(c: Counts, level: f64) -> Option<Ratio> {
    if c.n_pos <= 0.0 || c.n_neg <= 0.0 {
        return None;
    }
    let (a, m1, b, m0, corrected) = c.cells();
    let lr = (a / m1) / (b / m0);
    let se = (1.0 / a - 1.0 / m1 + 1.0 / b - 1.0 / m0).sqrt();
    let z = norm_ppf(0.5 + level / 2.0);
    Some(Ratio {
        lr,
        se,
        interval: (lr * (-z * se).exp(), lr * (z * se).exp()),
        corrected,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Fired fewer than `min_fired` times.
    Insufficient,
    /// The interval includes 1.
    Uninformative,
    /// The interval's lower end is at least `strong`.
    Strong,
    Informative,
}

/// Thresholds for the classes (configurable).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub min_fired: f64,
    pub strong: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            min_fired: 10.0,
            strong: 3.0,
        }
    }
}

pub fn classify(c: Counts, interval: (f64, f64), t: Thresholds) -> Class {
    if c.fired() < t.min_fired {
        Class::Insufficient
    } else if interval.0 <= 1.0 && 1.0 <= interval.1 {
        Class::Uninformative
    } else if interval.0 >= t.strong {
        Class::Strong
    } else {
        Class::Informative
    }
}

/// The posterior probability of positive after the feature fires: LR·o / (1 + LR·o)
/// with o = π / (1 − π).
pub fn posterior(lr: f64, prevalence: f64) -> f64 {
    let o = prevalence / (1.0 - prevalence);
    lr * o / (1.0 + lr * o)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Assessed {
    pub ratio: Ratio,
    /// Two-sided p-value of log LR = 0.
    pub p_value: f64,
    /// The interval at the BH-adjusted level, which the class is based on.
    pub adjusted_interval: (f64, f64),
    pub class: Class,
}

/// The BH-adjusted level for `m` features with p-values `ps` at error rate α:
/// 1 − max(R, 1)·α/m, R the number of BH rejections (Benjamini & Yekutieli 2005).
pub fn adjusted_level(ps: &[f64], alpha: f64) -> f64 {
    let m = ps.len().max(1) as f64;
    let mut sorted = ps.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rejected = sorted
        .iter()
        .enumerate()
        .filter(|(i, p)| **p <= (*i as f64 + 1.0) * alpha / m)
        .map(|(i, _)| i + 1)
        .max()
        .unwrap_or(0);
    1.0 - rejected.max(1) as f64 * alpha / m
}

/// Assess several features at once at confidence `level`.
pub fn assess(features: &[Counts], level: f64, t: Thresholds) -> Vec<Option<Assessed>> {
    let normal = Normal::standard();
    let ratios: Vec<Option<Ratio>> = features
        .iter()
        .map(|c| likelihood_ratio(*c, level))
        .collect();
    let ps: Vec<f64> = ratios
        .iter()
        .flatten()
        .map(|r| 2.0 * normal.sf(r.lr.ln().abs() / r.se))
        .collect();
    let adjusted = adjusted_level(&ps, 1.0 - level);
    features
        .iter()
        .zip(ratios)
        .map(|(c, r)| {
            let r = r?;
            let wide = likelihood_ratio(*c, adjusted)?;
            Some(Assessed {
                ratio: r,
                p_value: 2.0 * normal.sf(r.lr.ln().abs() / r.se),
                adjusted_interval: wide.interval,
                class: classify(*c, wide.interval, t),
            })
        })
        .collect()
}
