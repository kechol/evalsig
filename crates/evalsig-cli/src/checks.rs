//! Override flags for validity checks and how check findings enter an envelope.

use evalsig_core::checks::{self, Finding, Overrides};
use serde_json::json;

use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};

/// Flags for the checks that run over one dataset.
#[derive(Debug, Clone, Copy, Default, clap::Args, serde::Serialize, serde::Deserialize)]
pub struct DatasetArgs {
    /// Continue although some missing values have no explained reason.
    #[arg(long)]
    pub allow_unexplained_missing: bool,
    /// Stop when content looks like a secret instead of warning.
    #[arg(long)]
    pub fail_on_secret: bool,
}

/// Flags for the checks that compare two arms.
#[derive(Debug, Clone, Copy, Default, clap::Args, serde::Serialize, serde::Deserialize)]
pub struct ArmArgs {
    /// Continue although the compared arms used different grader versions.
    #[arg(long)]
    pub allow_mixed_grader: bool,
    /// Keep items whose input differs between the compared arms.
    #[arg(long)]
    pub allow_input_change: bool,
}

pub fn overrides(dataset: DatasetArgs, arms: ArmArgs) -> Overrides {
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
