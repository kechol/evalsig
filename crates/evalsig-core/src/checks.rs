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

/// A validity check: its code, summary and default action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CheckSpec {
    /// Stable code of the check.
    pub code: &'static str,
    /// One-line description of the condition the check detects.
    pub summary: &'static str,
    /// What the check does when it fires and no override is given.
    pub default: Action,
    /// The command-line flag that changes the default: it turns a stop or an exclusion
    /// into a warning, or a warning into a stop. `gate` never accepts it.
    pub override_flag: Option<&'static str>,
    /// What `gate` does instead, when it differs from the default.
    pub gate: Option<&'static str>,
}

const fn spec(
    code: &'static str,
    summary: &'static str,
    default: Action,
    override_flag: Option<&'static str>,
    gate: Option<&'static str>,
) -> CheckSpec {
    CheckSpec {
        code,
        summary,
        default,
        override_flag,
        gate,
    }
}

/// Every check of SPEC §7 in table order, with `comparison_graph_disconnected` from
/// metrics spec §10 after the other pairwise-preference checks.
pub const CHECKS: [CheckSpec; 23] = [
    spec(
        "missing_counted",
        "missing values are counted by reason and n_missing is in every output",
        Action::Always,
        None,
        None,
    ),
    spec(
        "unexplained_missing",
        "a missing value whose reason the importer could not determine",
        Action::Stop,
        Some("--allow-unexplained-missing"),
        Some("stop"),
    ),
    spec(
        "mixed_grader",
        "compared arms were graded by different grader versions",
        Action::Stop,
        Some("--allow-mixed-grader"),
        Some("stop (exit 3)"),
    ),
    spec(
        "unpaired_items",
        "compared arms cover different item sets; compared on the intersection, n_unpaired reported",
        Action::Report,
        None,
        Some(
            "a candidate that leaves holdout items out is refused (exit 3) before any budget is spent",
        ),
    ),
    spec(
        "input_changed",
        "the same item has different input content in compared arms; excluded, n_input_changed reported",
        Action::Exclude,
        Some("--allow-input-change"),
        Some("stop"),
    ),
    spec(
        "missing_rate_differs",
        "system-side missing rates differ between compared arms",
        Action::Warn,
        None,
        None,
    ),
    spec(
        "replicates_as_independent",
        "counting replicates as independent items",
        Action::Forbidden,
        None,
        None,
    ),
    spec(
        "mixed_output_type",
        "one run mixes output types or level counts",
        Action::Stop,
        None,
        Some("stop (exit 3)"),
    ),
    spec(
        "identical_replicates",
        "all generation replicates of an item have the same output (cache suspected)",
        Action::Warn,
        None,
        Some("stop"),
    ),
    spec(
        "run_variance_exceeds_delta",
        "the run-to-run standard deviation exceeds the point estimate of the difference",
        Action::Warn,
        None,
        None,
    ),
    spec(
        "split_crosses_cluster",
        "a split puts items of one cluster on both sides",
        Action::Stop,
        None,
        None,
    ),
    spec(
        "verification_bias_suspected",
        "human labels cover only some items and the inclusion probability is unknown",
        Action::Warn,
        None,
        None,
    ),
    spec(
        "position_bias",
        "in pairwise comparison the side shown first wins more often",
        Action::Warn,
        None,
        None,
    ),
    spec(
        "no_swapped_pairs",
        "pairwise comparisons have no position-swapped counterpart",
        Action::Warn,
        None,
        None,
    ),
    spec(
        "comparison_graph_disconnected",
        "the arms of a pairwise ranking do not form one connected comparison graph",
        Action::Warn,
        None,
        None,
    ),
    spec(
        "latent_reference_unidentified",
        "a latent reference has fewer than three raters per item; falls back to soft",
        Action::Warn,
        None,
        None,
    ),
    spec(
        "adjudication_in_agreement",
        "adjudicated labels are not independent and never enter agreement or parity tests",
        Action::Forbidden,
        None,
        None,
    ),
    spec(
        "gate_min_items",
        "the gate has fewer items than min_items; no decision is made (exit 4)",
        Action::Stop,
        None,
        Some("exit 4"),
    ),
    spec(
        "conditions_not_approved",
        "the conditions are not approved or changed after approval (exit 3)",
        Action::Stop,
        None,
        Some("exit 3; draft again and get approval"),
    ),
    spec(
        "budget_exhausted",
        "the holdout budget is spent (exit 2)",
        Action::Stop,
        None,
        Some("exit 2"),
    ),
    spec(
        "model_only_holdout_reference",
        "a holdout item's reference labels come only from models",
        Action::Report,
        None,
        Some("stop"),
    ),
    spec(
        "secret_in_content",
        "content looks like a secret (API key, private key)",
        Action::Warn,
        Some("--fail-on-secret"),
        None,
    ),
    spec(
        "model_derived_reference",
        "reference labels come from a model; stated at the top of the report",
        Action::Warn,
        None,
        None,
    ),
];

/// The specification of the check with this code, if there is one.
pub fn by_code(code: &str) -> Option<&'static CheckSpec> {
    CHECKS.iter().find(|c| c.code == code)
}

/// A check that fired.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// Code of the check that fired.
    pub code: &'static str,
    /// What was done about it.
    pub action: Action,
    /// Human-readable description of what was found.
    pub message: String,
    /// How to lift the check, when it can be lifted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Items affected, when counting them makes sense.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
}

impl Finding {
    /// A finding with the check's default action, changed by its override flag when
    /// the user passed it: a stop or an exclusion becomes a warning, a warning becomes
    /// a stop. Without the flag, the hint names it. `gate` passes no overrides.
    pub fn new(code: &'static str, message: impl Into<String>, overrides: &Overrides) -> Self {
        let spec = by_code(code).unwrap_or_else(|| panic!("unknown check code {code}"));
        let flag = spec.override_flag.filter(|f| overrides.has(f));
        let action = match (flag, spec.default) {
            (Some(_), Action::Stop | Action::Exclude) => Action::Warn,
            (Some(_), Action::Warn) => Action::Stop,
            _ => spec.default,
        };
        let hint = match (flag, spec.override_flag, action) {
            (None, Some(f), Action::Stop | Action::Exclude) => {
                Some(format!("pass {f} to continue anyway"))
            }
            (None, Some(f), Action::Warn) => Some(format!("pass {f} to stop instead")),
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

    /// Attach the number of affected items or records.
    pub fn with_count(mut self, n: u64) -> Self {
        self.count = Some(n);
        self
    }

    /// Whether the finding stops the command.
    pub fn stops(&self) -> bool {
        self.action == Action::Stop
    }
}

/// Override flags the user passed. `gate` uses the default, which passes none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Overrides {
    /// Turns the stop for a missing value of unexplained cause into a warning.
    pub allow_unexplained_missing: bool,
    /// Turns the stop for arms graded by different grader versions into a warning.
    pub allow_mixed_grader: bool,
    /// Compares items whose input changed between arms instead of excluding them.
    pub allow_input_change: bool,
    /// Turns the secret-in-content warning into a stop.
    pub fail_on_secret: bool,
}

impl Overrides {
    /// Whether the flag with this name (as in [`CheckSpec::override_flag`]) was passed.
    pub fn has(&self, flag: &str) -> bool {
        match flag {
            "--allow-unexplained-missing" => self.allow_unexplained_missing,
            "--allow-mixed-grader" => self.allow_mixed_grader,
            "--allow-input-change" => self.allow_input_change,
            "--fail-on-secret" => self.fail_on_secret,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC §7 row by row, with metrics spec §10's check after the pairwise ones: code,
    /// default action, override flag.
    const EXPECTED: [(&str, Action, Option<&str>); 23] = [
        ("missing_counted", Action::Always, None),
        (
            "unexplained_missing",
            Action::Stop,
            Some("--allow-unexplained-missing"),
        ),
        ("mixed_grader", Action::Stop, Some("--allow-mixed-grader")),
        ("unpaired_items", Action::Report, None),
        (
            "input_changed",
            Action::Exclude,
            Some("--allow-input-change"),
        ),
        ("missing_rate_differs", Action::Warn, None),
        ("replicates_as_independent", Action::Forbidden, None),
        ("mixed_output_type", Action::Stop, None),
        ("identical_replicates", Action::Warn, None),
        ("run_variance_exceeds_delta", Action::Warn, None),
        ("split_crosses_cluster", Action::Stop, None),
        ("verification_bias_suspected", Action::Warn, None),
        ("position_bias", Action::Warn, None),
        ("no_swapped_pairs", Action::Warn, None),
        ("comparison_graph_disconnected", Action::Warn, None),
        ("latent_reference_unidentified", Action::Warn, None),
        ("adjudication_in_agreement", Action::Forbidden, None),
        ("gate_min_items", Action::Stop, None),
        ("conditions_not_approved", Action::Stop, None),
        ("budget_exhausted", Action::Stop, None),
        ("model_only_holdout_reference", Action::Report, None),
        ("secret_in_content", Action::Warn, Some("--fail-on-secret")),
        ("model_derived_reference", Action::Warn, None),
    ];

    #[test]
    fn every_spec_check_is_registered_with_its_default_and_override() {
        assert_eq!(CHECKS.len(), EXPECTED.len());
        for (spec, (code, action, flag)) in CHECKS.iter().zip(EXPECTED) {
            assert_eq!(
                (spec.code, spec.default, spec.override_flag),
                (code, action, flag)
            );
        }
        let mut codes: Vec<_> = CHECKS.iter().map(|c| c.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), CHECKS.len(), "codes are unique");
    }

    #[test]
    fn flags_change_the_default_and_hints_name_them() {
        let stop = Finding::new("mixed_grader", "graders differ", &Overrides::default());
        assert!(stop.stops());
        assert_eq!(
            stop.hint.as_deref(),
            Some("pass --allow-mixed-grader to continue anyway")
        );
        let all = Overrides {
            allow_unexplained_missing: true,
            allow_mixed_grader: true,
            allow_input_change: true,
            fail_on_secret: true,
        };
        assert_eq!(Finding::new("mixed_grader", "x", &all).action, Action::Warn);
        assert_eq!(
            Finding::new("input_changed", "x", &all).action,
            Action::Warn
        );
        assert!(Finding::new("secret_in_content", "x", &all).stops());
        let warn = Finding::new("secret_in_content", "x", &Overrides::default());
        assert_eq!(
            warn.hint.as_deref(),
            Some("pass --fail-on-secret to stop instead")
        );
        // Checks without an override flag keep their default.
        assert!(Finding::new("mixed_output_type", "mixed", &all).stops());
    }

    #[test]
    fn every_registered_flag_is_recognized_and_off_by_default() {
        let all = Overrides {
            allow_unexplained_missing: true,
            allow_mixed_grader: true,
            allow_input_change: true,
            fail_on_secret: true,
        };
        for flag in CHECKS.iter().filter_map(|c| c.override_flag) {
            assert!(all.has(flag), "{flag}");
            assert!(!Overrides::default().has(flag), "{flag}");
        }
    }
}
