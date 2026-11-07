//! Meta-evaluation of a grader against the human reference (metrics spec §9.7): the
//! grader is the system and the reference built under §9.2 is the truth.
//!
//! Decision metrics (H, F, d′, cost) come from the report with the grader's run as
//! the system. This module adds the agreement view of the same pairs: κ, AC1 and the
//! specific agreements for two classes, the quadratic weighted κ and Spearman's ρ for
//! ordered values.

use serde::Serialize;

use crate::agreement;

/// Agreement between a grader and the reference labels (meta-evaluation).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct MetaEvaluation {
    /// Number of (grader, reference) pairs.
    pub n: usize,
    /// Cohen's κ; for nominal values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cohen_kappa: Option<f64>,
    /// Gwet's AC1; for nominal values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gwet_ac1: Option<f64>,
    /// Positive agreement 2a/(2a + b + c); for two classes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positive_agreement: Option<f64>,
    /// Negative agreement 2d/(2d + b + c); for two classes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_agreement: Option<f64>,
    /// Quadratic weighted κ; for ordered integer levels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quadratic_weighted_kappa: Option<f64>,
    /// Spearman's rank correlation ρ; for ordered values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spearman_rho: Option<f64>,
}

/// The integer levels that occur, ascending, and their table; `None` for non-integer
/// scores. The quadratic weights follow the level distance, so levels in between that
/// never occur need no row and the table is D×D for D distinct levels.
fn level_table(pairs: &[(f64, f64)]) -> Option<(Vec<f64>, Vec<Vec<f64>>)> {
    if pairs
        .iter()
        .flat_map(|&(a, b)| [a, b])
        .any(|v| v.fract() != 0.0)
    {
        return None;
    }
    Some(agreement::table(pairs))
}

/// Agreement of (grader, reference) pairs. `ordered` values get Spearman's ρ and,
/// for integer levels, the quadratic weighted κ; nominal values get κ and AC1, and two
/// classes also the positive and negative agreement with the higher value as positive.
pub fn evaluate(pairs: &[(f64, f64)], ordered: bool) -> MetaEvaluation {
    let mut m = MetaEvaluation {
        n: pairs.len(),
        ..MetaEvaluation::default()
    };
    if ordered {
        m.quadratic_weighted_kappa = level_table(pairs)
            .and_then(|(levels, t)| agreement::quadratic_weighted_kappa_at(&levels, &t));
        let (x, y): (Vec<f64>, Vec<f64>) = pairs.iter().copied().unzip();
        m.spearman_rho = agreement::spearman(&x, &y);
        return m;
    }
    let (mut categories, mut table) = agreement::table(pairs);
    // Booleans and decisions are coded 0/1: a pair that uses one class still has two.
    if let [c] = categories[..]
        && (c == 0.0 || c == 1.0)
    {
        let n = table[0][0];
        categories = vec![0.0, 1.0];
        table = vec![vec![0.0; 2]; 2];
        table[c as usize][c as usize] = n;
    }
    m.cohen_kappa = agreement::cohen_kappa(&table);
    m.gwet_ac1 = agreement::gwet_ac1(&table);
    if categories.len() == 2 {
        // `table` orders categories ascending; specific agreement wants positive first.
        let flipped: Vec<Vec<f64>> = table
            .iter()
            .rev()
            .map(|row| row.iter().rev().copied().collect())
            .collect();
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
    fn a_wide_range_of_levels_needs_no_table_over_the_range() {
        // Four distinct levels over a span of a million: the table is 4 × 4, and the
        // weights follow the distances. Reference values from numpy.
        let wide = evaluate(&[(0.0, 0.0), (1.0, 1.0), (1e6, 1e6), (1e6, 0.0)], true);
        assert!((wide.quadratic_weighted_kappa.unwrap() - 0.499_999_625_000_093_76).abs() < 1e-12);
        let near = evaluate(&[(0.0, 0.0), (1.0, 1.0), (5.0, 5.0), (5.0, 1.0)], true);
        assert!((near.quadratic_weighted_kappa.unwrap() - 0.594_936_708_860_759_6).abs() < 1e-12);
    }

    #[test]
    fn a_pair_that_uses_one_class_keeps_the_two_class_scale() {
        // Both raters say positive on every item: κ is undefined (p_e = 1), AC1 is 1, and
        // the positive agreement is 1 while the negative agreement is 0/0.
        let all_positive = evaluate(&[(1.0, 1.0); 40], false);
        assert_eq!(all_positive.cohen_kappa, None);
        assert_eq!(all_positive.gwet_ac1, Some(1.0));
        assert_eq!(all_positive.positive_agreement, Some(1.0));
        assert_eq!(all_positive.negative_agreement, None);
        let all_negative = evaluate(&[(0.0, 0.0); 40], false);
        assert_eq!(all_negative.gwet_ac1, Some(1.0));
        assert_eq!(all_negative.positive_agreement, None);
        assert_eq!(all_negative.negative_agreement, Some(1.0));
    }

    #[test]
    fn ordered_pairs_get_weighted_kappa_and_spearman() {
        let pairs = [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (3.0, 2.0)];
        let m = evaluate(&pairs, true);
        assert!(m.quadratic_weighted_kappa.unwrap() > 0.0);
        // Levels 1, 2 and 5: the unseen 3 and 4 still set the weights.
        let gap = evaluate(&[(1.0, 1.0), (2.0, 2.0), (5.0, 5.0), (5.0, 2.0)], true);
        let adjacent = evaluate(&[(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (3.0, 2.0)], true);
        assert!(gap.quadratic_weighted_kappa < adjacent.quadratic_weighted_kappa);
        assert_eq!(
            evaluate(&[(0.1, 0.2), (0.9, 0.8)], true).quadratic_weighted_kappa,
            None
        );
        // Midranks (1, 2, 3.5, 3.5) and (1, 2.5, 4, 2.5): ρ = 3.75 / 4.5.
        assert!((m.spearman_rho.unwrap() - 5.0 / 6.0).abs() < 1e-12);
        assert_eq!(m.cohen_kappa, None);
    }
}
