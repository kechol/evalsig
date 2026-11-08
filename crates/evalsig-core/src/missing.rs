//! Missing-value policy (metrics spec §0.1).
//!
//! System-side failures (the system errored, timed out, refused, produced invalid
//! output or hit a limit) default to the worst judgment, because they cluster on hard
//! items and dropping them would reward refusing. Measurement-side failures default to
//! being dropped from the denominator. Every result reports how many values were
//! missing and the policy used.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
            // Worst 0, best 1: a system-side gap takes the worst, the rest are dropped.
            let filled = policy.policy(r).fill(0.0, 1.0);
            let expected = (r.side() == Side::System).then_some(0.0);
            assert_eq!(filled, expected, "{r:?}");
        }
    }

    #[test]
    fn overridden_handling_for_each_reason() {
        for r in MissingReason::ALL {
            for (p, expected) in [
                (Policy::Worst, Some(0.0)),
                (Policy::Best, Some(1.0)),
                (Policy::Drop, None),
            ] {
                let policy = MissingPolicy::default().with_override(r, p);
                assert_eq!(policy.policy(r), p);
                assert_eq!(policy.policy(r).fill(0.0, 1.0), expected, "{r:?} {p:?}");
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
        }
        assert!(policy.table().values().all(|p| *p == Policy::Worst));
    }

    #[test]
    fn worst_depends_on_the_metric_direction() {
        // For a false alarm rate the worst judgment is a false alarm (1).
        assert_eq!(Policy::Worst.fill(1.0, 0.0), Some(1.0));
        assert_eq!(Policy::Best.fill(1.0, 0.0), Some(0.0));
    }
}
