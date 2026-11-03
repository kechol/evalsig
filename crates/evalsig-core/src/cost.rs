//! Expected cost and decision thresholds (metrics spec §4).
//!
//! Levels count from the weakest (0). The cost matrix has a row per reference label
//! (negative, positive) and a column per judgment level; `prevalence` is the
//! production prevalence π, not the test set's share. Counts may be weighted, so a soft
//! reference counts item i as s_i positive and 1 − s_i negative.

use crate::numeric::sum;

fn shares(counts: &[f64]) -> Option<Vec<f64>> {
    let total = sum(counts.iter().copied());
    (total > 0.0).then(|| counts.iter().map(|c| c / total).collect())
}

/// EC = π Σ_j P(j|pos) C[1][j] + (1 − π) Σ_j P(j|neg) C[0][j].
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
pub fn optimal_threshold(
    pos_by_level: &[f64],
    neg_by_level: &[f64],
    matrix: &[[f64; 2]; 2],
    prevalence: f64,
) -> Option<(usize, f64)> {
    threshold_costs(pos_by_level, neg_by_level, matrix, prevalence)?
        .into_iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The matrix with C_FP = 1, C_FN = r and no cost for correct decisions.
pub fn ratio_matrix(r: f64) -> [[f64; 2]; 2] {
    [[0.0, 1.0], [r, 0.0]]
}

/// The optimal threshold at both ends of the cost-ratio range r = C_FN / C_FP.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct ThresholdSensitivity {
    pub at_low: usize,
    pub at_high: usize,
    pub changes: bool,
}

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
    pub at_low: f64,
    pub at_high: f64,
    pub sign_changes: bool,
}

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
    DifferenceSensitivity {
        at_low,
        at_high,
        // A tie at either end is no change of sign.
        sign_changes: at_low * at_high < 0.0,
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
