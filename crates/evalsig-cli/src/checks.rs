//! Override flags for validity checks, shared by every command that runs them, and
//! how check findings enter an envelope.

use evalsig_core::checks::{self, Action, Finding, Overrides};
use evalsig_core::missing::MissingCounts;
use evalsig_core::records::Record;
use serde_json::json;

use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};

#[derive(Debug, Clone, Copy, Default, clap::Args)]
pub struct CheckArgs {
    /// Continue although some missing values have no explained reason.
    #[arg(long)]
    pub allow_unexplained_missing: bool,
    /// Continue although the compared arms used different grader versions.
    #[arg(long)]
    pub allow_mixed_grader: bool,
    /// Keep items whose input differs between the compared arms.
    #[arg(long)]
    pub allow_input_change: bool,
    /// Stop when content looks like a secret instead of warning.
    #[arg(long)]
    pub fail_on_secret: bool,
}

impl CheckArgs {
    pub fn overrides(self) -> Overrides {
        Overrides {
            allow_unexplained_missing: self.allow_unexplained_missing,
            allow_mixed_grader: self.allow_mixed_grader,
            allow_input_change: self.allow_input_change,
            fail_on_secret: self.fail_on_secret,
        }
    }
}

/// Missing judgments by reason, reported in every output that reads judgments.
pub fn missing_counts(records: &[Record]) -> MissingCounts {
    let mut c = MissingCounts::default();
    for r in records {
        if let Record::Judgment(j) = r {
            match &j.missing {
                Some(m) => {
                    c.n_missing += 1;
                    *c.missing_by_reason.entry(m.reason).or_default() += 1;
                }
                None => c.n_used += 1,
            }
        }
    }
    c
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
    if !checks::stops(findings) {
        return Output::success(envelope);
    }
    let stopped: Vec<&str> = findings
        .iter()
        .filter(|f| f.action == Action::Stop)
        .map(|f| f.code)
        .collect();
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
