//! The gate's rules (SPEC §8.3 "Rules", metrics spec §11.1). Every rule reads an
//! interval or a verdict; none reads a point estimate alone. A delta is
//! candidate − baseline on the metric's own scale.

use serde::{Deserialize, Serialize};

use crate::interval::{clopper_pearson, clopper_pearson_upper};

/// A condition rule of a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// The candidate value's interval has a lower bound of at least `value`.
    LowerBoundAtLeast,
    /// The candidate value's interval has an upper bound of at most `value`.
    UpperBoundAtMost,
    /// The delta's interval has a lower bound of at least `value`.
    DeltaLowerBoundAtLeast,
    /// The delta's interval has an upper bound of at most `value`.
    DeltaUpperBoundAtMost,
    /// Non-inferiority: the delta's lower bound is at least `value` (a negative margin).
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
    /// The difference between the arms with its confidence interval.
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

/// The result of evaluating a rule on its evidence.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Outcome {
    /// `None` when the evidence does not fit the rule or is missing.
    pub passed: Option<bool>,
    /// The bound the rule read.
    pub bound: Option<f64>,
}

impl Rule {
    /// Whether the rule reads the delta rather than the candidate's value.
    pub fn on_delta(self) -> bool {
        matches!(
            self,
            Rule::DeltaLowerBoundAtLeast | Rule::DeltaUpperBoundAtMost | Rule::NotWorseThan
        )
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

/// Judge one condition. `min_effect` applies to the two delta rules: the point
/// difference must move by at least that much in the rule's direction.
pub fn judge(
    rule: Rule,
    value: f64,
    min_effect: Option<f64>,
    evidence: Option<Evidence>,
) -> Outcome {
    let none = Outcome {
        passed: None,
        bound: None,
    };
    let Some(e) = evidence else { return none };
    let m = min_effect.unwrap_or(0.0);
    let (passed, bound) = match (rule, e) {
        (Rule::LowerBoundAtLeast, Evidence::Value { lo, .. }) => (lo >= value, lo),
        (Rule::UpperBoundAtMost, Evidence::Value { hi, .. }) => (hi <= value, hi),
        (Rule::DeltaLowerBoundAtLeast, Evidence::Delta { point, lo, .. }) => {
            (lo >= value && point >= m, lo)
        }
        (Rule::DeltaUpperBoundAtMost, Evidence::Delta { point, hi, .. }) => {
            (hi <= value && point <= -m, hi)
        }
        (Rule::NotWorseThan, Evidence::Delta { lo, .. }) => (lo >= value, lo),
        (Rule::Pass, Evidence::Verdict(v)) => {
            return Outcome {
                passed: Some(v),
                bound: None,
            };
        }
        _ => return none,
    };
    Outcome {
        passed: Some(passed),
        bound: Some(bound),
    }
}

/// All conditions must hold (an intersection-union test, Berger 1982): one failure
/// fails, otherwise an undecided condition leaves the gate undecided.
pub fn overall(outcomes: &[Outcome]) -> Option<bool> {
    if outcomes.iter().any(|o| o.passed == Some(false)) {
        Some(false)
    } else if outcomes.iter().any(|o| o.passed.is_none()) {
        None
    } else {
        Some(true)
    }
}

/// The missing rate on the holdout, every missing judgment counted whatever its
/// reason, with the exact interval at the condition's two-sided `interval_level`
/// (one-sided conditions read its upper end).
pub fn missing_rate(
    missing: u64,
    judgments: u64,
    interval_level: f64,
    sided: Sided,
) -> Option<Evidence> {
    if judgments == 0 {
        return None;
    }
    let (k, n) = (missing as f64, judgments as f64);
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
    let k = value / step;
    let k = if value >= baseline {
        k.floor()
    } else {
        k.ceil()
    };
    k * step
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
            judge(Rule::LowerBoundAtLeast, 0.8, None, Some(wide)).passed,
            Some(false)
        );
        // Evidence of the wrong kind or none: undecided.
        assert_eq!(
            judge(Rule::NotWorseThan, -0.05, None, Some(wide)).passed,
            None
        );
        assert_eq!(judge(Rule::Pass, 0.0, None, None).passed, None);
        assert_eq!(
            overall(&[
                Outcome {
                    passed: Some(true),
                    bound: None
                },
                Outcome {
                    passed: None,
                    bound: None
                }
            ]),
            None
        );
        assert!((interval_level(0.95, 5, Sided::One) - 0.98).abs() < 1e-12);
        assert!((interval_level(0.95, 5, Sided::Two) - 0.99).abs() < 1e-12);
    }
}
