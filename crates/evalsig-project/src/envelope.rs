//! The output envelope shared by every command (D-008, agent-interface spec §2.2):
//! its fields, verdict codes and provenance. The binary renders it.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::exit::Exit;

/// The conclusion of an output. The set is fixed within a schema version; agents
/// quote unknown codes without interpreting them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictCode {
    Pass,
    Fail,
    Improved,
    Negligible,
    Worsened,
    Indistinguishable,
    Drifted,
    Stable,
    HumanParity,
    BelowHuman,
    InsufficientData,
    Blocked,
    NeedsDecisions,
    BudgetExhausted,
}

impl VerdictCode {
    pub const ALL: [VerdictCode; 14] = [
        VerdictCode::Pass,
        VerdictCode::Fail,
        VerdictCode::Improved,
        VerdictCode::Negligible,
        VerdictCode::Worsened,
        VerdictCode::Indistinguishable,
        VerdictCode::Drifted,
        VerdictCode::Stable,
        VerdictCode::HumanParity,
        VerdictCode::BelowHuman,
        VerdictCode::InsufficientData,
        VerdictCode::Blocked,
        VerdictCode::NeedsDecisions,
        VerdictCode::BudgetExhausted,
    ];

    /// What the code means and what may be said about it (agent-interface spec §2.3).
    pub fn meaning(self) -> &'static str {
        match self {
            VerdictCode::Pass => "every approved condition holds; say the conditions were met",
            VerdictCode::Fail => "a condition does not hold; say which condition was not met",
            VerdictCode::Improved => {
                "the interval of the difference excludes 0 on the better side and the point difference reaches the minimum effect; say it improved, with the interval"
            }
            VerdictCode::Negligible => {
                "the interval lies within the minimum effect, or the difference is real but small; say there is no practical difference, neither improved nor worsened"
            }
            VerdictCode::Worsened => {
                "the interval of the difference excludes 0 on the worse side; say it worsened, with the interval"
            }
            VerdictCode::Indistinguishable => {
                "none of the above: the interval crosses 0; say the difference cannot be told apart, neither improved nor worsened"
            }
            VerdictCode::Drifted => {
                "over time, a period's interval left the baseline period's; say which period and the interval"
            }
            VerdictCode::Stable => "over time, no period left the baseline period",
            VerdictCode::HumanParity => {
                "the system agrees with people as well as people do (alternative annotator test); quote the winning rate"
            }
            VerdictCode::BelowHuman => "the system does not agree with people as well as people do",
            VerdictCode::InsufficientData => {
                "too few items to decide; quote the required count from the power command in next"
            }
            VerdictCode::Blocked => {
                "a validity check or a referee check stopped the command; quote the reason and how to clear it (the gate has no override)"
            }
            VerdictCode::NeedsDecisions => {
                "a mapping draft has decisions a person must make; show the choices and their effects and ask, without choosing"
            }
            VerdictCode::BudgetExhausted => {
                "the holdout's budget is spent; say so and ask a person to decide on the budget"
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            VerdictCode::Pass => "pass",
            VerdictCode::Fail => "fail",
            VerdictCode::Improved => "improved",
            VerdictCode::Negligible => "negligible",
            VerdictCode::Worsened => "worsened",
            VerdictCode::Indistinguishable => "indistinguishable",
            VerdictCode::Drifted => "drifted",
            VerdictCode::Stable => "stable",
            VerdictCode::HumanParity => "human_parity",
            VerdictCode::BelowHuman => "below_human",
            VerdictCode::InsufficientData => "insufficient_data",
            VerdictCode::Blocked => "blocked",
            VerdictCode::NeedsDecisions => "needs_decisions",
            VerdictCode::BudgetExhausted => "budget_exhausted",
        }
    }
}

impl From<evalsig_core::paired::Verdict> for VerdictCode {
    fn from(v: evalsig_core::paired::Verdict) -> Self {
        use evalsig_core::paired::Verdict as V;
        match v {
            V::Improved => VerdictCode::Improved,
            V::Negligible => VerdictCode::Negligible,
            V::Worsened => VerdictCode::Worsened,
            V::Indistinguishable => VerdictCode::Indistinguishable,
            V::InsufficientData => VerdictCode::InsufficientData,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub code: VerdictCode,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Next {
    pub command: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Provenance {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub snapshots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    pub tool: String,
    /// The metric definitions the results follow (`evalsig_core::SPEC_VERSION`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub spec_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_seq: Option<u64>,
    /// The command and options that produced the results, so `verify` can recompute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Json>,
    /// Hash of the canonical results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl Provenance {
    pub fn tool_only() -> Self {
        Provenance {
            tool: crate::tool_version(),
            spec_version: evalsig_core::SPEC_VERSION.into(),
            ..Provenance::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Truncated {
    pub rows_omitted: u64,
    /// Strings cut to the cell limit.
    #[serde(default)]
    pub cells_cut: u64,
    /// The `--cursor` that shows the next rows, when rows remain after these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<usize>,
    /// The BLAKE3 reference of the canonical JSON of the full `results`.
    pub hash: String,
}

/// The envelope. `ok` says the command ran to the end, not whether a gate passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub schema: String,
    pub ok: bool,
    pub verdict: Option<Verdict>,
    pub results: Json,
    pub warnings: Vec<Warning>,
    pub next: Vec<Next>,
    pub provenance: Provenance,
    pub truncated: Option<Truncated>,
    /// Text from the evaluated data, only on request in agent mode (D-018).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untrusted_content: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_notice: Option<String>,
}

impl Envelope {
    pub fn new(schema: &str, results: impl Serialize) -> Self {
        Envelope {
            schema: schema.to_owned(),
            ok: true,
            verdict: None,
            results: serde_json::to_value(results).expect("results serialize to JSON"),
            warnings: Vec::new(),
            next: Vec::new(),
            provenance: Provenance::tool_only(),
            truncated: None,
            untrusted_content: None,
            content_notice: None,
        }
    }
}

/// What a command produced: the envelope and the exit code it implies.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub envelope: Envelope,
    pub exit: Exit,
}

impl Output {
    pub fn success(envelope: Envelope) -> Self {
        Output {
            envelope,
            exit: Exit::Success,
        }
    }
}

/// Round every number to 10 decimal places (metrics spec §0).
pub fn round_numbers(v: Json) -> Json {
    match v {
        Json::Number(n) if n.is_f64() => {
            let x = evalsig_store::canonical::round_output(n.as_f64().expect("f64 number"));
            serde_json::Number::from_f64(x).map_or(Json::Null, Json::Number)
        }
        Json::Array(xs) => Json::Array(xs.into_iter().map(round_numbers).collect()),
        Json::Object(m) => {
            Json::Object(m.into_iter().map(|(k, v)| (k, round_numbers(v))).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_verdicts_keep_their_names() {
        use evalsig_core::paired::Verdict as V;
        for v in [
            V::Improved,
            V::Negligible,
            V::Worsened,
            V::Indistinguishable,
            V::InsufficientData,
        ] {
            assert_eq!(
                serde_json::to_value(v).unwrap(),
                VerdictCode::from(v).as_str()
            );
        }
    }

    #[test]
    fn verdict_names_match_their_serialization() {
        for c in VerdictCode::ALL {
            assert_eq!(serde_json::to_value(c).unwrap(), c.as_str());
        }
    }
}
