//! The output envelope shared by every command (D-008, agent-interface spec §2.2):
//! its fields, verdict codes and provenance. The binary renders it.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::exit::Exit;

/// Every envelope schema the commands write. A schema version changes when a field's
/// meaning changes or a field is removed (D-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schema {
    Agree,
    Capabilities,
    Check,
    Compare,
    Exclude,
    Explain,
    Export,
    Gate,
    Help,
    Holdout,
    Import,
    Index,
    Init,
    Items,
    LedgerAnchor,
    LedgerShow,
    LedgerVerify,
    MapDecide,
    MapDraft,
    MapProbe,
    MapTest,
    Migrate,
    Power,
    Preregister,
    Purge,
    Recipes,
    Referee,
    Remaining,
    Report,
    Runs,
    Schema,
    Seal,
    Skill,
    Split,
    Splits,
    Threshold,
    Trend,
    Validate,
    Verify,
    Version,
}

impl Schema {
    pub const ALL: [Schema; 40] = [
        Schema::Agree,
        Schema::Capabilities,
        Schema::Check,
        Schema::Compare,
        Schema::Exclude,
        Schema::Explain,
        Schema::Export,
        Schema::Gate,
        Schema::Help,
        Schema::Holdout,
        Schema::Import,
        Schema::Index,
        Schema::Init,
        Schema::Items,
        Schema::LedgerAnchor,
        Schema::LedgerShow,
        Schema::LedgerVerify,
        Schema::MapDecide,
        Schema::MapDraft,
        Schema::MapProbe,
        Schema::MapTest,
        Schema::Migrate,
        Schema::Power,
        Schema::Preregister,
        Schema::Purge,
        Schema::Recipes,
        Schema::Referee,
        Schema::Remaining,
        Schema::Report,
        Schema::Runs,
        Schema::Schema,
        Schema::Seal,
        Schema::Skill,
        Schema::Split,
        Schema::Splits,
        Schema::Threshold,
        Schema::Trend,
        Schema::Validate,
        Schema::Verify,
        Schema::Version,
    ];

    /// The name an envelope's `schema` field carries.
    pub fn name(self) -> &'static str {
        match self {
            Schema::Agree => "evalsig.agree/1",
            Schema::Capabilities => "evalsig.capabilities/1",
            Schema::Check => "evalsig.check/1",
            Schema::Compare => "evalsig.compare/1",
            Schema::Exclude => "evalsig.exclude/1",
            Schema::Explain => "evalsig.explain/1",
            Schema::Export => "evalsig.export/1",
            Schema::Gate => "evalsig.gate/1",
            Schema::Help => "evalsig.help/1",
            Schema::Holdout => "evalsig.holdout/1",
            Schema::Import => "evalsig.import/1",
            Schema::Index => "evalsig.index/1",
            Schema::Init => "evalsig.init/1",
            Schema::Items => "evalsig.items/1",
            Schema::LedgerAnchor => "evalsig.ledger.anchor/1",
            Schema::LedgerShow => "evalsig.ledger.show/1",
            Schema::LedgerVerify => "evalsig.ledger.verify/1",
            Schema::MapDecide => "evalsig.map.decide/1",
            Schema::MapDraft => "evalsig.map.draft/1",
            Schema::MapProbe => "evalsig.map.probe/1",
            Schema::MapTest => "evalsig.map.test/1",
            Schema::Migrate => "evalsig.migrate/1",
            Schema::Power => "evalsig.power/1",
            Schema::Preregister => "evalsig.preregister/1",
            Schema::Purge => "evalsig.purge/1",
            Schema::Recipes => "evalsig.recipes/1",
            Schema::Referee => "evalsig.referee/1",
            Schema::Remaining => "evalsig.remaining/1",
            Schema::Report => "evalsig.report/1",
            Schema::Runs => "evalsig.runs/1",
            Schema::Schema => "evalsig.schema/1",
            Schema::Seal => "evalsig.seal/1",
            Schema::Skill => "evalsig.skill/1",
            Schema::Split => "evalsig.split/1",
            Schema::Splits => "evalsig.splits/1",
            Schema::Threshold => "evalsig.threshold/1",
            Schema::Trend => "evalsig.trend/1",
            Schema::Validate => "evalsig.validate/1",
            Schema::Verify => "evalsig.verify/1",
            Schema::Version => "evalsig.version/1",
        }
    }

    /// The fields whose text comes from the evaluated data, wherever they occur in
    /// `results`. Agent mode takes them out (D-018); identifiers (item, run, rater,
    /// system) stay, since every answer needs them.
    pub fn content_fields(self) -> &'static [&'static str] {
        match self {
            Schema::Compare => &["by"],
            // Issue messages quote the offending values; paths can hold the data's own keys.
            Schema::Import => &["message", "path"],
            Schema::Items => &["strata"],
            Schema::MapDraft => &["values"],
            Schema::MapProbe => &["values"],
            Schema::MapTest => &["values", "invalid_examples", "message", "got", "expected"],
            // Per-stratum results are keyed by the strata's own values.
            Schema::Report => &["by"],
            Schema::Validate => &["message", "path"],
            // A mismatch echoes the verified results, content included.
            Schema::Verify => &["saved", "recomputed", "path"],
            Schema::Agree
            | Schema::Capabilities
            | Schema::Check
            | Schema::Exclude
            | Schema::Explain
            | Schema::Export
            | Schema::Gate
            | Schema::Help
            | Schema::Holdout
            | Schema::Index
            | Schema::Init
            | Schema::LedgerAnchor
            | Schema::LedgerShow
            | Schema::LedgerVerify
            | Schema::MapDecide
            | Schema::Migrate
            | Schema::Power
            | Schema::Preregister
            | Schema::Purge
            | Schema::Recipes
            | Schema::Referee
            | Schema::Remaining
            | Schema::Runs
            | Schema::Schema
            | Schema::Seal
            | Schema::Skill
            | Schema::Split
            | Schema::Splits
            | Schema::Threshold
            | Schema::Trend
            | Schema::Version => &[],
        }
    }

    /// The schema of this name, if the commands write one.
    pub fn from_name(name: &str) -> Option<Schema> {
        Schema::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// The conclusion of an output. The set is fixed within a schema version; agents
/// quote unknown codes without interpreting them. Serialized as [`VerdictCode::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl Serialize for VerdictCode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VerdictCode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        VerdictCode::ALL
            .into_iter()
            .find(|c| c.as_str() == name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown verdict code {name:?}")))
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Pretty JSON with a final newline, as results are printed and saved.
    pub fn to_pretty_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("envelope serializes");
        s.push('\n');
        s
    }

    pub fn new(schema: Schema, results: impl Serialize) -> Self {
        Envelope {
            schema: schema.name().to_owned(),
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
    fn verdict_codes_read_back() {
        for c in VerdictCode::ALL {
            let v = serde_json::to_value(c).unwrap();
            assert_eq!(serde_json::from_value::<VerdictCode>(v).unwrap(), c);
        }
        assert!(serde_json::from_value::<VerdictCode>("maybe".into()).is_err());
    }

    #[test]
    fn schema_names_are_distinct_and_read_back() {
        let mut names: Vec<&str> = Schema::ALL.iter().map(|s| s.name()).collect();
        for s in Schema::ALL {
            assert_eq!(Schema::from_name(s.name()), Some(s));
            assert!(s.name().starts_with("evalsig.") && s.name().ends_with("/1"));
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Schema::ALL.len());
    }
}
