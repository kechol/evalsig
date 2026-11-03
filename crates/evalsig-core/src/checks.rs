//! Validity checks (SPEC §7): measurement errors evalsig stops by default.
//!
//! Every check has a stable code, a default action, the flag that overrides it (if
//! any) and the task that delivers it. `gate` accepts no override flag at all.

use serde::Serialize;

/// What a check does by default when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Applied to every output; nothing to fire.
    Always,
    /// Stop the command (exit code 5, or the stated gate exit code).
    Stop,
    /// Report a warning and continue.
    Warn,
    /// Leave the affected items out and report how many.
    Exclude,
    /// Analyse what is comparable and report the count of the rest.
    Report,
    /// The configuration that would trigger it cannot be expressed.
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CheckSpec {
    pub code: &'static str,
    pub summary: &'static str,
    pub default: Action,
    /// The command-line flag that lifts the check; `gate` never accepts it.
    pub override_flag: Option<&'static str>,
    /// What `gate` does instead, when it differs from the default.
    pub gate: Option<&'static str>,
    /// The backlog task that implements the check.
    pub delivered_by: &'static str,
}

const fn spec(
    code: &'static str,
    summary: &'static str,
    default: Action,
    override_flag: Option<&'static str>,
    gate: Option<&'static str>,
    delivered_by: &'static str,
) -> CheckSpec {
    CheckSpec {
        code,
        summary,
        default,
        override_flag,
        gate,
        delivered_by,
    }
}

/// Every check of SPEC §7, in table order.
pub const CHECKS: [CheckSpec; 22] = [
    spec(
        "missing_counted",
        "missing values are counted by reason and n_missing is in every output",
        Action::Always,
        None,
        None,
        "T-038",
    ),
    spec(
        "unexplained_missing",
        "a missing value whose reason the importer could not determine",
        Action::Stop,
        Some("--allow-unexplained-missing"),
        Some("stop"),
        "T-039",
    ),
    spec(
        "mixed_grader",
        "compared arms were graded by different grader versions",
        Action::Stop,
        Some("--allow-mixed-grader"),
        Some("stop (exit 3)"),
        "T-039",
    ),
    spec(
        "unpaired_items",
        "compared arms cover different item sets; compared on the intersection, n_unpaired reported",
        Action::Report,
        None,
        Some("absent holdout items are filled with the worst judgment"),
        "T-039",
    ),
    spec(
        "input_changed",
        "the same item has different input content in compared arms; excluded, n_input_changed reported",
        Action::Exclude,
        Some("--allow-input-change"),
        Some("stop"),
        "T-039",
    ),
    spec(
        "missing_rate_differs",
        "system-side missing rates differ between compared arms",
        Action::Warn,
        None,
        None,
        "T-039",
    ),
    spec(
        "replicates_as_independent",
        "counting replicates as independent items",
        Action::Forbidden,
        None,
        None,
        "T-038",
    ),
    spec(
        "mixed_output_type",
        "one run mixes output types or level counts",
        Action::Stop,
        None,
        Some("stop (exit 3)"),
        "T-039",
    ),
    spec(
        "identical_replicates",
        "all generation replicates of an item have the same output (cache suspected)",
        Action::Warn,
        None,
        Some("stop"),
        "T-039",
    ),
    spec(
        "run_variance_exceeds_delta",
        "the run-to-run standard deviation exceeds the point estimate of the difference",
        Action::Warn,
        None,
        None,
        "T-039",
    ),
    spec(
        "split_crosses_cluster",
        "a split puts items of one cluster on both sides",
        Action::Stop,
        None,
        None,
        "T-083",
    ),
    spec(
        "verification_bias_suspected",
        "human labels cover only some items and the inclusion probability is unknown",
        Action::Warn,
        None,
        None,
        "T-039",
    ),
    spec(
        "position_bias",
        "in pairwise comparison the side shown first wins more often",
        Action::Warn,
        None,
        None,
        "T-058",
    ),
    spec(
        "no_swapped_pairs",
        "pairwise comparisons have no position-swapped counterpart",
        Action::Warn,
        None,
        None,
        "T-058",
    ),
    spec(
        "latent_reference_unidentified",
        "a latent reference has fewer than three raters per item; falls back to soft",
        Action::Warn,
        None,
        None,
        "T-055",
    ),
    spec(
        "adjudication_in_agreement",
        "adjudicated labels are not independent and never enter agreement or parity tests",
        Action::Forbidden,
        None,
        None,
        "T-039",
    ),
    spec(
        "gate_min_items",
        "the gate has fewer items than min_items; no decision is made (exit 4)",
        Action::Stop,
        None,
        Some("exit 4"),
        "T-125",
    ),
    spec(
        "conditions_not_approved",
        "the conditions are not approved or changed after approval (exit 3)",
        Action::Stop,
        None,
        Some("exit 3; draft again and get approval"),
        "T-086",
    ),
    spec(
        "budget_exhausted",
        "the holdout budget is spent (exit 2)",
        Action::Stop,
        None,
        Some("exit 2"),
        "T-125",
    ),
    spec(
        "model_only_holdout_reference",
        "a holdout item's reference labels come only from models",
        Action::Report,
        None,
        Some("stop"),
        "T-125",
    ),
    spec(
        "secret_in_content",
        "content looks like a secret (API key, private key)",
        Action::Warn,
        Some("--fail-on-secret"),
        None,
        "T-039",
    ),
    spec(
        "model_derived_reference",
        "reference labels come from a model; stated at the top of the report",
        Action::Warn,
        None,
        None,
        "T-039",
    ),
];

pub fn by_code(code: &str) -> Option<&'static CheckSpec> {
    CHECKS.iter().find(|c| c.code == code)
}

/// A check that fired.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub action: Action,
    pub message: String,
    /// How to lift the check, when it can be lifted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Items affected, when counting them makes sense.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
}

impl Finding {
    /// A finding with the check's default action, or `Warn` when the user passed its
    /// override flag (overrides never apply inside `gate`).
    pub fn new(code: &'static str, message: impl Into<String>, overridden: bool) -> Self {
        let spec = by_code(code).unwrap_or_else(|| panic!("unknown check code {code}"));
        let action = if overridden && spec.override_flag.is_some() {
            Action::Warn
        } else {
            spec.default
        };
        let hint = match (action, spec.override_flag) {
            (Action::Stop | Action::Exclude, Some(flag)) => {
                Some(format!("pass {flag} to continue anyway"))
            }
            _ => None,
        };
        Finding {
            code,
            action,
            message: message.into(),
            hint,
            count: None,
        }
    }

    pub fn with_count(mut self, n: u64) -> Self {
        self.count = Some(n);
        self
    }

    pub fn stops(&self) -> bool {
        self.action == Action::Stop
    }
}

/// Override flags the user passed. `gate` constructs [`Overrides::none`] and offers no
/// way to set them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Overrides {
    pub allow_unexplained_missing: bool,
    pub allow_mixed_grader: bool,
    pub allow_input_change: bool,
    pub fail_on_secret: bool,
}

impl Overrides {
    pub const fn none() -> Self {
        Overrides {
            allow_unexplained_missing: false,
            allow_mixed_grader: false,
            allow_input_change: false,
            fail_on_secret: false,
        }
    }

    /// Whether the user lifted the check with its flag.
    pub fn lifts(&self, code: &str) -> bool {
        match code {
            "unexplained_missing" => self.allow_unexplained_missing,
            "mixed_grader" => self.allow_mixed_grader,
            "input_changed" => self.allow_input_change,
            _ => false,
        }
    }
}

/// The outcome of running checks: whether anything stops the command.
pub fn stops(findings: &[Finding]) -> bool {
    findings.iter().any(Finding::stops)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC §7, row by row: code, default action, override flag, delivering task.
    const EXPECTED: [(&str, Action, Option<&str>, &str); 22] = [
        ("missing_counted", Action::Always, None, "T-038"),
        (
            "unexplained_missing",
            Action::Stop,
            Some("--allow-unexplained-missing"),
            "T-039",
        ),
        (
            "mixed_grader",
            Action::Stop,
            Some("--allow-mixed-grader"),
            "T-039",
        ),
        ("unpaired_items", Action::Report, None, "T-039"),
        (
            "input_changed",
            Action::Exclude,
            Some("--allow-input-change"),
            "T-039",
        ),
        ("missing_rate_differs", Action::Warn, None, "T-039"),
        (
            "replicates_as_independent",
            Action::Forbidden,
            None,
            "T-038",
        ),
        ("mixed_output_type", Action::Stop, None, "T-039"),
        ("identical_replicates", Action::Warn, None, "T-039"),
        ("run_variance_exceeds_delta", Action::Warn, None, "T-039"),
        ("split_crosses_cluster", Action::Stop, None, "T-083"),
        ("verification_bias_suspected", Action::Warn, None, "T-039"),
        ("position_bias", Action::Warn, None, "T-058"),
        ("no_swapped_pairs", Action::Warn, None, "T-058"),
        ("latent_reference_unidentified", Action::Warn, None, "T-055"),
        (
            "adjudication_in_agreement",
            Action::Forbidden,
            None,
            "T-039",
        ),
        ("gate_min_items", Action::Stop, None, "T-125"),
        ("conditions_not_approved", Action::Stop, None, "T-086"),
        ("budget_exhausted", Action::Stop, None, "T-125"),
        (
            "model_only_holdout_reference",
            Action::Report,
            None,
            "T-125",
        ),
        (
            "secret_in_content",
            Action::Warn,
            Some("--fail-on-secret"),
            "T-039",
        ),
        ("model_derived_reference", Action::Warn, None, "T-039"),
    ];

    #[test]
    fn every_spec_check_is_registered_with_its_default_override_and_task() {
        assert_eq!(CHECKS.len(), EXPECTED.len());
        for (spec, (code, action, flag, task)) in CHECKS.iter().zip(EXPECTED) {
            assert_eq!(
                (
                    spec.code,
                    spec.default,
                    spec.override_flag,
                    spec.delivered_by
                ),
                (code, action, flag, task)
            );
        }
        let mut codes: Vec<_> = CHECKS.iter().map(|c| c.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), CHECKS.len(), "codes are unique");
    }

    #[test]
    fn overrides_turn_a_stop_into_a_warning_with_a_hint_otherwise() {
        let stop = Finding::new("mixed_grader", "graders differ", false);
        assert!(stop.stops());
        assert_eq!(
            stop.hint.as_deref(),
            Some("pass --allow-mixed-grader to continue anyway")
        );
        let lifted = Finding::new(
            "mixed_grader",
            "graders differ",
            Overrides {
                allow_mixed_grader: true,
                ..Overrides::none()
            }
            .lifts("mixed_grader"),
        );
        assert_eq!(lifted.action, Action::Warn);
        // Checks without an override flag ignore the request.
        assert!(Finding::new("mixed_output_type", "mixed", true).stops());
        assert!(stops(&[stop]));
    }

    #[test]
    fn gate_overrides_lift_nothing() {
        let none = Overrides::none();
        for c in CHECKS {
            assert!(!none.lifts(c.code));
        }
    }
}
