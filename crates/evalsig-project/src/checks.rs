//! Overrides of validity checks and how check findings enter an envelope.

use evalsig_core::checks::{self, Finding, Overrides};
use serde_json::json;

use crate::envelope::{Envelope, Output, Verdict, VerdictCode, Warning};
use crate::exit::Exit;

/// Overrides of the checks that run over one dataset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DatasetChecks {
    /// Continue although some missing values have no explained reason.
    pub allow_unexplained_missing: bool,
    /// Stop when content looks like a secret instead of warning.
    pub fail_on_secret: bool,
}

/// Overrides of the checks that compare two arms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArmChecks {
    /// Continue although the compared arms used different grader versions.
    pub allow_mixed_grader: bool,
    /// Keep items whose input differs between the compared arms.
    pub allow_input_change: bool,
}

pub fn overrides(dataset: DatasetChecks, arms: ArmChecks) -> Overrides {
    Overrides {
        allow_unexplained_missing: dataset.allow_unexplained_missing,
        allow_mixed_grader: arms.allow_mixed_grader,
        allow_input_change: arms.allow_input_change,
        fail_on_secret: dataset.fail_on_secret,
    }
}

/// Put findings into an envelope: every finding is a warning; a stopping finding
/// sets verdict `blocked` and exit code 5.
pub fn apply(mut envelope: Envelope, findings: &[Finding]) -> Output {
    envelope.warnings.extend(findings.iter().map(|f| Warning {
        code: f.code.to_owned(),
        message: match &f.hint {
            Some(h) => format!("{} ({h})", f.message),
            None => f.message.clone(),
        },
    }));
    let stopped: Vec<&str> = findings
        .iter()
        .filter(|f| f.stops())
        .map(|f| f.code)
        .collect();
    if stopped.is_empty() {
        return Output::success(envelope);
    }
    envelope.verdict = Some(Verdict {
        code: VerdictCode::Blocked,
        text: format!("Stopped by validity checks: {}.", stopped.join(", ")),
    });
    Output {
        envelope,
        exit: Exit::Blocked,
    }
}

/// The registry, for `validate --list-checks`.
pub fn registry() -> serde_json::Value {
    json!(checks::CHECKS)
}
