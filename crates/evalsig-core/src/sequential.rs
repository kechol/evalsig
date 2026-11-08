//! Confidence sequences for a bounded mean (metrics spec §7.2): intervals that hold
//! simultaneously at every sample size, so the data may be inspected after each item.
//!
//! The predictable plug-in empirical-Bernstein sequence of Waudby-Smith & Ramdas (2024,
//! Theorem 2) for observations in [0, 1]:
//!
//! - μ̂_t = (½ + Σ_{i≤t} x_i)/(t + 1) and σ̂²_t = (¼ + Σ_{i≤t} (x_i − μ̂_i)²)/(t + 1);
//! - λ_t = min(√(2 log(2/α) / (σ̂²_{t−1} · t · log(1 + t))), ½);
//! - v_t = 4(x_t − μ̂_{t−1})² and ψ(λ) = (−log(1 − λ) − λ)/4;
//! - C_t = Σλ_i x_i / Σλ_i ± (log(2/α) + Σ v_i ψ(λ_i)) / Σλ_i,
//!
//! intersected with [0, 1] and with every earlier interval.

use crate::numeric::Running;

/// The largest bet; Waudby-Smith & Ramdas suggest ½ or ¾.
const MAX_BET: f64 = 0.5;

/// The sequence of intervals after each observation (each in [0, 1]) at confidence
/// `level`.
pub fn bounded_mean(xs: &[f64], level: f64) -> Vec<(f64, f64)> {
    let log_term = (2.0 / (1.0 - level)).ln();
    let [mut sum_x, mut sum_sq] = [Running::default(); 2];
    let (mut mu_prev, mut var_prev) = (0.5, 0.25);
    let [mut sum_lambda, mut sum_lambda_x, mut sum_penalty] = [Running::default(); 3];
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    let mut out = Vec::with_capacity(xs.len());
    for (k, &x) in xs.iter().enumerate() {
        let t = (k + 1) as f64;
        let lambda = (2.0 * log_term / (var_prev * t * (1.0 + t).ln()))
            .sqrt()
            .min(MAX_BET);
        let v = 4.0 * (x - mu_prev).powi(2);
        let psi = (-(1.0 - lambda).ln() - lambda) / 4.0;
        sum_lambda.add(lambda);
        sum_lambda_x.add(lambda * x);
        sum_penalty.add(v * psi);
        let centre = sum_lambda_x.total() / sum_lambda.total();
        let radius = (log_term + sum_penalty.total()) / sum_lambda.total();
        lo = lo.max(centre - radius);
        hi = hi.min(centre + radius);
        out.push((lo, hi.max(lo)));
        sum_x.add(x);
        let mu = (0.5 + sum_x.total()) / (t + 1.0);
        sum_sq.add((x - mu).powi(2));
        mu_prev = mu;
        var_prev = (0.25 + sum_sq.total()) / (t + 1.0);
    }
    out
}

/// The sequence for the mean of differences in [−1, 1], through x = (d + 1)/2.
pub fn difference(ds: &[f64], level: f64) -> Vec<(f64, f64)> {
    let xs: Vec<f64> = ds
        .iter()
        .map(|d| (d.clamp(-1.0, 1.0) + 1.0) / 2.0)
        .collect();
    bounded_mean(&xs, level)
        .into_iter()
        .map(|(lo, hi)| (2.0 * lo - 1.0, 2.0 * hi - 1.0))
        .collect()
}
