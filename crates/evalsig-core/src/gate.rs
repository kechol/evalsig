//! The gate's rules (SPEC §8.3 "Rules", metrics spec §11.1). Every rule reads an
//! interval or a verdict; none reads a point estimate alone. A difference is the
//! candidate's against the baseline's, oriented so that positive is better whatever the
//! metric's direction (D-034).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::interval::{clopper_pearson, clopper_pearson_upper};
use crate::numeric::{round_output, sum};
use crate::records::Judgment;

/// A condition rule of a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// The candidate value's interval has a lower bound of at least `value`.
    LowerBoundAtLeast,
    /// The candidate value's interval has an upper bound of at most `value`.
    UpperBoundAtMost,
    /// Superiority by a margin: the difference's lower bound is above `value` (at least
    /// 0), and its point reaches `min_effect`.
    BetterBy,
    /// Non-inferiority: the difference's lower bound is above −`value`, the margin (at
    /// least 0) by which the candidate may trail.
    NotWorseThan,
    /// A verdict metric such as `human_parity`.
    Pass,
}

/// Whether the interval is one-sided or two-sided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sided {
    /// One-sided interval.
    One,
    /// Two-sided interval.
    Two,
}

/// What a rule reads: the candidate's value with its interval, the delta with its
/// interval, or a verdict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Evidence {
    /// The candidate's value with its confidence interval.
    Value {
        /// Point estimate.
        point: f64,
        /// Lower end of the interval.
        lo: f64,
        /// Upper end of the interval.
        hi: f64,
    },
    /// The difference between the arms, oriented so that positive is better, with its
    /// confidence interval.
    Delta {
        /// Point estimate.
        point: f64,
        /// Lower end of the interval.
        lo: f64,
        /// Upper end of the interval.
        hi: f64,
    },
    /// A verdict metric's pass or fail.
    Verdict(bool),
}

impl Rule {
    /// Whether the rule reads the delta rather than the candidate's value.
    pub fn on_delta(self) -> bool {
        matches!(self, Rule::BetterBy | Rule::NotWorseThan)
    }
}

/// The two-sided interval level whose ends give the condition's bounds when each
/// submission is judged at 1 − α/B (α = 1 − `level`).
pub fn interval_level(level: f64, budget: u32, sided: Sided) -> f64 {
    let alpha = (1.0 - level) / f64::from(budget.max(1));
    match sided {
        Sided::One => 1.0 - 2.0 * alpha,
        Sided::Two => 1.0 - alpha,
    }
}

/// The end of the interval a rule reads: the lower end, or the upper end for
/// `upper_bound_at_most`. `None` for `pass` and for evidence the rule does not read.
pub fn bound(rule: Rule, evidence: Evidence) -> Option<f64> {
    match (rule, evidence) {
        (Rule::LowerBoundAtLeast, Evidence::Value { lo, .. }) => Some(lo),
        (Rule::UpperBoundAtMost, Evidence::Value { hi, .. }) => Some(hi),
        (Rule::BetterBy | Rule::NotWorseThan, Evidence::Delta { lo, .. }) => Some(lo),
        _ => None,
    }
}

/// Judge one condition. The difference rules are strict, so a candidate no different from
/// the baseline (an interval of zero width at 0) does not pass `better_by = 0`.
/// `min_effect` applies to `better_by`: the point difference must reach it. `None` when
/// the evidence is missing or does not fit the rule.
pub fn judge(
    rule: Rule,
    value: f64,
    min_effect: Option<f64>,
    evidence: Option<Evidence>,
) -> Option<bool> {
    let e = evidence?;
    if let (Rule::Pass, Evidence::Verdict(v)) = (rule, e) {
        return Some(v);
    }
    let b = bound(rule, e)?;
    Some(match (rule, e) {
        (Rule::LowerBoundAtLeast, _) => b >= value,
        (Rule::UpperBoundAtMost, _) => b <= value,
        (Rule::BetterBy, Evidence::Delta { point, .. }) => {
            b > value && point >= min_effect.unwrap_or(0.0)
        }
        (Rule::NotWorseThan, _) => b > -value,
        _ => return None,
    })
}

/// All conditions must hold (an intersection-union test, Berger 1982): one failure
/// fails, otherwise an undecided condition leaves the gate undecided.
pub fn overall(outcomes: &[Option<bool>]) -> Option<bool> {
    if outcomes.contains(&Some(false)) {
        Some(false)
    } else if outcomes.contains(&None) {
        None
    } else {
        Some(true)
    }
}

/// The events and the size of the missing rate's exact interval: the item is the
/// counting unit (metrics spec §0), so an item contributes its share of missing
/// judgments and replicates never add to the count. `counts` says which judgments
/// are missing.
pub fn missing_counts(judgments: &[&Judgment], counts: impl Fn(&Judgment) -> bool) -> (f64, f64) {
    let mut per_item: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for j in judgments {
        let (missing, all) = per_item.entry(j.item_id.as_str()).or_default();
        *missing += f64::from(u8::from(counts(j)));
        *all += 1.0;
    }
    let shares = per_item.values().map(|(missing, all)| missing / all);
    (sum(shares), per_item.len() as f64)
}

/// The missing rate on the holdout from `missing_counts`, with the exact interval at
/// the condition's two-sided `interval_level` (one-sided conditions read its upper
/// end).
pub fn missing_rate(
    missing: f64,
    items: f64,
    interval_level: f64,
    sided: Sided,
) -> Option<Evidence> {
    if items <= 0.0 {
        return None;
    }
    let (k, n) = (missing, items);
    let (lo, hi) = match sided {
        // The upper end of the two-sided interval at 2L − 1 is the one-sided bound at L.
        Sided::One => (
            0.0,
            clopper_pearson_upper(k, n, (1.0 + interval_level) / 2.0),
        ),
        Sided::Two => clopper_pearson(k, n, interval_level),
    };
    Some(Evidence::Value {
        point: k / n,
        lo,
        hi,
    })
}

/// The Ladder's expected share of items two systems answer differently (metrics spec
/// §11.3): ψ̂ = max(0.05, 4/n, min(1, 2·e_val)).
pub fn ladder_psi(e_val: f64, n: f64) -> f64 {
    (2.0 * e_val).min(1.0).max(4.0 / n).max(0.05)
}

/// The Ladder's step η = max(k·√(ψ̂/n), 1/n), n the primary condition's denominator.
pub fn ladder_step(k: f64, psi: f64, n: f64) -> f64 {
    (k * (psi / n).sqrt()).max(1.0 / n)
}

/// A value rounded to a multiple of `step` toward `baseline`, so that rounding never
/// makes the candidate look better than it is.
pub fn round_toward(value: f64, baseline: f64, step: f64) -> f64 {
    // A value on the grid stays there whatever the last bit of the division says.
    const TOLERANCE: f64 = 1e-9;
    let k = value / step;
    let k = if value >= baseline {
        (k + TOLERANCE).floor()
    } else {
        (k - TOLERANCE).ceil()
    };
    round_output(k * step)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_step_reproduces_the_spec_example() {
        // e_val = 0.1 and 300 items in the denominator give η ≈ 0.05.
        let eta = ladder_step(2.0, ladder_psi(0.1, 300.0), 300.0);
        assert!((eta - 2.0 * (0.2_f64 / 300.0).sqrt()).abs() < 1e-12);
        assert!((eta - 0.0516).abs() < 1e-4, "{eta}");
        // Few items: the floors of ψ̂ and η apply.
        assert_eq!(ladder_psi(0.0, 300.0), 0.05);
        // n = 4: ψ̂ = 4/n = 1, so η = √(1/4) = 0.5.
        assert_eq!(ladder_step(1.0, ladder_psi(0.0, 4.0), 4.0), 0.5);
    }

    #[test]
    fn rounding_moves_toward_the_baseline() {
        assert!((round_toward(0.837, 0.80, 0.05) - 0.80).abs() < 1e-12);
        assert!((round_toward(0.763, 0.80, 0.05) - 0.80).abs() < 1e-12);
        assert!((round_toward(0.912, 0.80, 0.05) - 0.90).abs() < 1e-12);
        // A difference rounds toward 0.
        assert!((round_toward(-0.07, 0.0, 0.05) + 0.05).abs() < 1e-12);
        // A value on the grid keeps its step, and the result carries no float noise.
        assert_eq!(round_toward(0.3, 0.0, 0.1), 0.3);
        assert_eq!(round_toward(0.85, 0.5, 0.05), 0.85);
        // The grid value is written as the output rounding writes it, ties included.
        let step = 0.154_436_278_848_75;
        assert_eq!(round_toward(6.18, 0.0, step), round_output(40.0 * step));
        assert_eq!(round_toward(6.18, 0.0, step), 6.177_451_153_9);
    }

    #[test]
    fn rules_never_pass_on_a_point_alone() {
        // A good point with a wide interval does not pass any interval rule.
        let wide = Evidence::Value {
            point: 0.9,
            lo: 0.4,
            hi: 1.0,
        };
        assert_eq!(
            judge(Rule::LowerBoundAtLeast, 0.8, None, Some(wide)),
            Some(false)
        );
        // Evidence of the wrong kind or none: undecided.
        assert_eq!(judge(Rule::NotWorseThan, 0.05, None, Some(wide)), None);
        assert_eq!(judge(Rule::Pass, 0.0, None, None), None);
        assert_eq!(overall(&[Some(true), None]), None);
        assert!((interval_level(0.95, 5, Sided::One) - 0.98).abs() < 1e-12);
        // A candidate identical to the baseline gives a zero-width interval at 0: it is
        // not better by 0, though it is not worse than a margin.
        let same = Evidence::Delta {
            point: 0.0,
            lo: 0.0,
            hi: 0.0,
        };
        assert_eq!(judge(Rule::BetterBy, 0.0, None, Some(same)), Some(false));
        assert_eq!(
            judge(Rule::NotWorseThan, 0.02, None, Some(same)),
            Some(true)
        );
        assert_eq!(
            judge(Rule::NotWorseThan, 0.0, None, Some(same)),
            Some(false)
        );
        let all = Evidence::Delta {
            point: 1.0,
            lo: 1.0,
            hi: 1.0,
        };
        assert_eq!(judge(Rule::BetterBy, 0.0, None, Some(all)), Some(true));
        assert!((interval_level(0.95, 5, Sided::Two) - 0.99).abs() < 1e-12);
    }
}
