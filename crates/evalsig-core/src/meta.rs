//! Meta-evaluation of a grader against the human reference (metrics spec §9.7): the
//! grader is the system and the reference built under §9.2 is the truth.
//!
//! Decision metrics (H, F, d′, cost) come from the report with the grader's run as
//! the system. This module adds the agreement view of the same pairs: κ, AC1 and the
//! specific agreements for two classes, the quadratic weighted κ and Spearman's ρ for
//! ordered values.

use serde::Serialize;

use crate::agreement;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct MetaEvaluation {
    pub n: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cohen_kappa: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gwet_ac1: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positive_agreement: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_agreement: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quadratic_weighted_kappa: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spearman_rho: Option<f64>,
}

/// Agreement of (grader, reference) pairs. `ordered` values (levels, scores) get the
/// weighted κ and Spearman's ρ; nominal values get κ and AC1, and two classes also
/// the positive and negative agreement with the higher value as positive.
pub fn evaluate(pairs: &[(f64, f64)], ordered: bool) -> MetaEvaluation {
    let (categories, table) = agreement::table(pairs);
    let mut m = MetaEvaluation {
        n: pairs.len(),
        ..MetaEvaluation::default()
    };
    if ordered {
        m.quadratic_weighted_kappa = agreement::quadratic_weighted_kappa(&table);
        let (x, y): (Vec<f64>, Vec<f64>) = pairs.iter().copied().unzip();
        m.spearman_rho = agreement::spearman(&x, &y);
        return m;
    }
    m.cohen_kappa = agreement::cohen_kappa(&table);
    m.gwet_ac1 = agreement::gwet_ac1(&table);
    if categories.len() == 2 {
        // `table` orders categories ascending; specific agreement wants positive first.
        let flipped = vec![
            vec![table[1][1], table[1][0]],
            vec![table[0][1], table[0][0]],
        ];
        if let Some((pos, neg)) = agreement::specific_agreement(&flipped) {
            m.positive_agreement = pos;
            m.negative_agreement = neg;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_pairs_get_kappa_and_specific_agreement() {
        // grader, reference: 3 both positive, 1 grader-only, 1 reference-only, 5 both negative.
        let mut pairs = vec![(1.0, 1.0); 3];
        pairs.push((1.0, 0.0));
        pairs.push((0.0, 1.0));
        pairs.extend([(0.0, 0.0); 5]);
        let m = evaluate(&pairs, false);
        assert_eq!(m.n, 10);
        assert_eq!(m.positive_agreement, Some(6.0 / 8.0));
        assert_eq!(m.negative_agreement, Some(10.0 / 12.0));
        // po 0.8, pe 0.4·0.4 + 0.6·0.6 = 0.52.
        assert!((m.cohen_kappa.unwrap() - (0.8 - 0.52) / 0.48).abs() < 1e-12);
        assert_eq!(m.spearman_rho, None);
    }

    #[test]
    fn ordered_pairs_get_weighted_kappa_and_spearman() {
        let pairs = [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (3.0, 2.0)];
        let m = evaluate(&pairs, true);
        assert!(m.quadratic_weighted_kappa.unwrap() > 0.0);
        // Midranks (1, 2, 3.5, 3.5) and (1, 2.5, 4, 2.5): ρ = 3.75 / 4.5.
        assert!((m.spearman_rho.unwrap() - 5.0 / 6.0).abs() < 1e-12);
        assert_eq!(m.cohen_kappa, None);
    }
}
