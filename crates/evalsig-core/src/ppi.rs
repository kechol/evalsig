//! Prediction-powered inference for a mean (PPI++, Angelopoulos et al. 2023;
//! metrics spec §9.8).
//!
//! A proxy (an LLM grader) labels every item; humans label a random subset. With the
//! n labelled pairs (y_i, f_i) and the proxy's values on the N items without a human
//! label, θ̂ = λ·mean_N(f) + mean_n(y − λf). The variance is λ²·Var_N(f)/N +
//! Var_n(y − λf)/n and the interval is the normal one. λ defaults to the variance-
//! minimising Cov_n(y, f) / ((1 + n/N)·Var(f)), clipped to [0, 1]; λ = 0 uses the human
//! labels alone and λ = 1 is the original PPI. The conventions (population covariance
//! on the labelled items, sample variance of f over all items, population variances in
//! the standard error) follow `ppi_py`, which the reference case reproduces.

use serde::Serialize;

use crate::numeric::{mean, sum, two_sided_z, variance};

/// The PPI++ estimate of a mean with its normal interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Ppi {
    /// The prediction-powered point estimate θ̂.
    pub estimate: f64,
    /// Standard error of the estimate.
    pub se: f64,
    /// Normal confidence interval of the estimate.
    pub interval: (f64, f64),
    /// The weight λ on the proxy, in [0, 1].
    pub lambda: f64,
    /// Number of items with a human label.
    pub labelled: usize,
    /// Number of items without a human label.
    pub unlabelled: usize,
}

/// The PPI++ estimate of the mean of y. `labelled` holds (y, f) pairs, `unlabelled`
/// the proxy's values elsewhere. `lambda` fixes λ; `None` uses the optimal one.
/// `None` without at least two labelled and two unlabelled items.
pub fn mean_estimate(
    labelled: &[(f64, f64)],
    unlabelled: &[f64],
    level: f64,
    lambda: Option<f64>,
) -> Option<Ppi> {
    let (n, big_n) = (labelled.len(), unlabelled.len());
    if n < 2 || big_n < 2 {
        return None;
    }
    let y: Vec<f64> = labelled.iter().map(|p| p.0).collect();
    let f: Vec<f64> = labelled.iter().map(|p| p.1).collect();
    let lambda = lambda.unwrap_or_else(|| {
        let (my, mf) = (
            mean(y.iter().copied()).unwrap_or(0.0),
            mean(f.iter().copied()).unwrap_or(0.0),
        );
        let cov = sum(y.iter().zip(&f).map(|(a, b)| (a - my) * (b - mf))) / n as f64;
        let all: Vec<f64> = f.iter().chain(unlabelled).copied().collect();
        let var_f = variance(&all, 1);
        let denom = (1.0 + n as f64 / big_n as f64) * var_f;
        if denom > 0.0 {
            (cov / denom).clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    let rectifier: Vec<f64> = y.iter().zip(&f).map(|(a, b)| a - lambda * b).collect();
    let estimate = lambda * mean(unlabelled.iter().copied())? + mean(rectifier.iter().copied())?;
    let se = (lambda * lambda * variance(unlabelled, 0) / big_n as f64
        + variance(&rectifier, 0) / n as f64)
        .sqrt();
    let z = two_sided_z(level);
    Some(Ppi {
        estimate,
        se,
        interval: (estimate - z * se, estimate + z * se),
        lambda,
        labelled: n,
        unlabelled: big_n,
    })
}
