//! Missing-value policy (metrics spec §0.1).
//!
//! System-side failures (the system errored, timed out, refused, produced invalid
//! output or hit a limit) default to the worst judgment, because they cluster on hard
//! items and dropping them would reward refusing. Measurement-side failures default to
//! being dropped from the denominator. Every result reports how many values were
//! missing and the policy used.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::numeric::Running;
use crate::records::MissingReason;

/// Which side of the measurement failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The system under test failed to produce a judgment.
    System,
    /// The measurement failed: the grader, the run or the data pipeline.
    Measurement,
}

impl MissingReason {
    /// The side that the reason code blames.
    pub fn side(self) -> Side {
        match self {
            MissingReason::Error
            | MissingReason::Timeout
            | MissingReason::Refused
            | MissingReason::InvalidOutput
            | MissingReason::Limit => Side::System,
            MissingReason::GraderFailed
            | MissingReason::NotRun
            | MissingReason::Invalidated
            | MissingReason::Excluded
            | MissingReason::Other => Side::Measurement,
        }
    }

    /// The handling used when the configuration does not override it.
    pub fn default_policy(self) -> Policy {
        match self.side() {
            Side::System => Policy::Worst,
            Side::Measurement => Policy::Drop,
        }
    }
}

/// How a missing value enters a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// Count it as the worst judgment for the metric (metrics spec §0.1.1).
    Worst,
    /// Count it as the best judgment for the metric.
    Best,
    /// Remove it from the denominator.
    Drop,
}

impl Policy {
    /// The value a missing observation takes; `None` removes it from the denominator.
    pub fn fill(self, worst: f64, best: f64) -> Option<f64> {
        match self {
            Policy::Worst => Some(worst),
            Policy::Best => Some(best),
            Policy::Drop => None,
        }
    }
}

/// The policy in force for each reason.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MissingPolicy {
    overrides: BTreeMap<MissingReason, Policy>,
    /// Holdout gates do not trust reason codes written by the candidate's side.
    distrust_reasons: bool,
}

impl MissingPolicy {
    /// The policy a holdout gate uses: every missing value is the worst judgment.
    pub fn holdout() -> Self {
        MissingPolicy {
            overrides: BTreeMap::new(),
            distrust_reasons: true,
        }
    }

    /// Set the handling for one reason (from the `[missing]` configuration).
    pub fn with_override(mut self, reason: MissingReason, policy: Policy) -> Self {
        self.overrides.insert(reason, policy);
        self
    }

    /// How a missing value with this reason is handled.
    pub fn policy(&self, reason: MissingReason) -> Policy {
        if self.distrust_reasons {
            return Policy::Worst;
        }
        self.overrides
            .get(&reason)
            .copied()
            .unwrap_or_else(|| reason.default_policy())
    }

    /// The resolved policy for every reason, as reported in `missing_policy`.
    pub fn table(&self) -> BTreeMap<MissingReason, Policy> {
        MissingReason::ALL
            .into_iter()
            .map(|r| (r, self.policy(r)))
            .collect()
    }
}

/// A value, or the reason it is missing.
pub type Observed<T> = Result<T, MissingReason>;

/// Counts reported next to every metric.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MissingCounts {
    /// Observations entering the denominator, including filled ones.
    pub n_used: u64,
    /// Missing observations, filled or dropped.
    pub n_missing: u64,
    /// Missing observations counted per reason.
    pub missing_by_reason: BTreeMap<MissingReason, u64>,
}

/// Values and missing markers across the judgments, labels and preferences of a
/// dataset, before any policy applies.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Tally {
    /// Observations that have a value.
    pub values: u64,
    /// Observations marked missing.
    pub n_missing: u64,
    /// Missing observations counted per reason.
    pub missing_by_reason: BTreeMap<MissingReason, u64>,
}

/// Count the values and the missing markers of the records.
pub fn tally(records: &[crate::records::Record]) -> Tally {
    use crate::records::Record;
    let mut t = Tally::default();
    for r in records {
        if !matches!(
            r,
            Record::Judgment(_) | Record::Label(_) | Record::Preference(_)
        ) {
            continue;
        }
        match r.missing() {
            Some(m) => {
                t.n_missing += 1;
                *t.missing_by_reason.entry(m.reason).or_default() += 1;
            }
            None => t.values += 1,
        }
    }
    t
}

/// A weighted mean under a missing policy, with Manski bounds when system-side
/// values are missing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeanWithMissing {
    /// `None` when nothing is left in the denominator.
    pub point: Option<f64>,
    /// System-side missing values filled with the best and the worst value, other
    /// reasons handled by the policy. Present only if a system-side value is missing.
    pub bounds: Option<(f64, f64)>,
    /// Counts of observations used and missing.
    pub counts: MissingCounts,
}

/// Weighted mean of per-item values in `[worst, best]` order of preference, where a
/// missing value is filled or dropped by `policy`. `worst` and `best` are the values
/// the metric assigns to the worst and best possible judgment (for a rate, 0 and 1).
pub fn mean_with_missing(
    values: &[(Observed<f64>, f64)],
    policy: &MissingPolicy,
    worst: f64,
    best: f64,
) -> MeanWithMissing {
    let mut counts = MissingCounts::default();
    let mut point = Acc::default();
    let mut low = Acc::default();
    let mut high = Acc::default();
    let mut system_missing = false;
    for &(obs, w) in values {
        match obs {
            Ok(v) => {
                counts.n_used += 1;
                for acc in [&mut point, &mut low, &mut high] {
                    acc.add(v, w);
                }
            }
            Err(reason) => {
                counts.n_missing += 1;
                *counts.missing_by_reason.entry(reason).or_default() += 1;
                let filled = policy.policy(reason).fill(worst, best);
                if let Some(v) = filled {
                    counts.n_used += 1;
                    point.add(v, w);
                }
                if reason.side() == Side::System {
                    system_missing = true;
                    low.add(worst, w);
                    high.add(best, w);
                } else if let Some(v) = filled {
                    low.add(v, w);
                    high.add(v, w);
                }
            }
        }
    }
    let bounds = system_missing.then(|| {
        let (a, b) = (low.mean().unwrap_or(worst), high.mean().unwrap_or(best));
        (a.min(b), a.max(b))
    });
    MeanWithMissing {
        point: point.mean(),
        bounds,
        counts,
    }
}

#[derive(Default)]
struct Acc {
    sum: Running,
    weight: Running,
}

impl Acc {
    fn add(&mut self, v: f64, w: f64) {
        self.sum.add(v * w);
        self.weight.add(w);
    }

    fn mean(&self) -> Option<f64> {
        let w = self.weight.total();
        (w > 0.0).then(|| self.sum.total() / w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEM: [MissingReason; 5] = [
        MissingReason::Error,
        MissingReason::Timeout,
        MissingReason::Refused,
        MissingReason::InvalidOutput,
        MissingReason::Limit,
    ];

    /// Three correct, one wrong, one missing with `reason`.
    fn sample(reason: MissingReason) -> Vec<(Observed<f64>, f64)> {
        vec![
            (Ok(1.0), 1.0),
            (Ok(1.0), 1.0),
            (Ok(1.0), 1.0),
            (Ok(0.0), 1.0),
            (Err(reason), 1.0),
        ]
    }

    #[test]
    fn sides_and_defaults_cover_all_ten_reasons() {
        for r in MissingReason::ALL {
            let system = SYSTEM.contains(&r);
            assert_eq!(
                r.side(),
                if system {
                    Side::System
                } else {
                    Side::Measurement
                }
            );
            assert_eq!(
                r.default_policy(),
                if system { Policy::Worst } else { Policy::Drop }
            );
        }
    }

    #[test]
    fn default_handling_for_each_reason() {
        let policy = MissingPolicy::default();
        for r in MissingReason::ALL {
            let m = mean_with_missing(&sample(r), &policy, 0.0, 1.0);
            assert_eq!(m.counts.n_missing, 1);
            assert_eq!(m.counts.missing_by_reason.get(&r), Some(&1));
            if r.side() == Side::System {
                assert_eq!(m.point, Some(3.0 / 5.0), "{r:?}");
                assert_eq!(m.counts.n_used, 5);
                assert_eq!(m.bounds, Some((3.0 / 5.0, 4.0 / 5.0)));
            } else {
                assert_eq!(m.point, Some(3.0 / 4.0), "{r:?}");
                assert_eq!(m.counts.n_used, 4);
                assert_eq!(m.bounds, None);
            }
        }
    }

    #[test]
    fn overridden_handling_for_each_reason() {
        for r in MissingReason::ALL {
            for (p, expected, used) in [
                (Policy::Worst, 3.0 / 5.0, 5),
                (Policy::Best, 4.0 / 5.0, 5),
                (Policy::Drop, 3.0 / 4.0, 4),
            ] {
                let policy = MissingPolicy::default().with_override(r, p);
                assert_eq!(policy.policy(r), p);
                let m = mean_with_missing(&sample(r), &policy, 0.0, 1.0);
                assert_eq!(m.point, Some(expected), "{r:?} {p:?}");
                assert_eq!(m.counts.n_used, used);
                // Bounds always span best and worst for system-side reasons.
                if r.side() == Side::System {
                    assert_eq!(m.bounds, Some((3.0 / 5.0, 4.0 / 5.0)));
                } else if p == Policy::Drop {
                    assert_eq!(m.bounds, None);
                }
            }
            // Other reasons keep their defaults.
            let policy = MissingPolicy::default().with_override(r, Policy::Best);
            for other in MissingReason::ALL.into_iter().filter(|o| *o != r) {
                assert_eq!(policy.policy(other), other.default_policy());
            }
        }
    }

    #[test]
    fn holdout_policy_ignores_reasons_and_overrides() {
        let policy = MissingPolicy::holdout();
        for r in MissingReason::ALL {
            assert_eq!(policy.policy(r), Policy::Worst);
            let m = mean_with_missing(&sample(r), &policy, 0.0, 1.0);
            assert_eq!(m.point, Some(3.0 / 5.0));
        }
        assert!(policy.table().values().all(|p| *p == Policy::Worst));
    }

    #[test]
    fn worst_depends_on_the_metric_direction() {
        // For a false alarm rate, the worst judgment is a false alarm (1).
        let values = [(Ok(0.0), 1.0), (Err(MissingReason::Timeout), 1.0)];
        let m = mean_with_missing(&values, &MissingPolicy::default(), 1.0, 0.0);
        assert_eq!(m.point, Some(0.5));
        assert_eq!(m.bounds, Some((0.0, 0.5)));
    }

    #[test]
    fn weights_and_empty_denominators() {
        let values = [(Ok(1.0), 3.0), (Ok(0.0), 1.0)];
        let m = mean_with_missing(&values, &MissingPolicy::default(), 0.0, 1.0);
        assert_eq!(m.point, Some(0.75));
        let only_dropped = [(Err(MissingReason::NotRun), 1.0)];
        let m = mean_with_missing(&only_dropped, &MissingPolicy::default(), 0.0, 1.0);
        assert_eq!(m.point, None);
        assert_eq!(m.counts.n_used, 0);
    }
}
