//! Gate conditions (`gate.toml`, SPEC §8.3) as a draft and as the referee checks them
//! again at approval, since the agent can write the stored copy.

use std::collections::BTreeMap;
use std::path::Path;

use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::gate::{Rule as GateRule, Sided};
use evalsig_core::missing::Policy;
use evalsig_core::records::{MissingReason, OutputType, ValueSemantics};
use evalsig_core::rules::Rule;
use evalsig_project::Error;
use evalsig_project::project::io_error;
use evalsig_project::schemas;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Gate conditions (`gate/1`): a draft, or, with `resolved`, the approved copy the
/// gate reads. Every optional field is left out when unset, since TOML has no null
/// and the approved file must read back to the document its hash names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    pub name: String,
    pub hypothesis: String,
    pub split: SplitKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub compare: Arms,
    pub grader: GraderPin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positive: Option<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimand: Option<Estimand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replicates: Option<u64>,
    /// Missing handling for validation; on the holdout every missing value is the worst.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<BTreeMap<MissingReason, Policy>>,
    pub min_items: u64,
    pub conditions: Vec<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceDraft>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holdout: Option<HoldoutDraft>,
    /// Carried as written; the gate does not read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Value>,
    /// Carried as written; approval pins the mapping it is given, not this name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping: Option<String>,
    /// Written by approval only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitKind {
    Validation,
    Holdout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arms {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    pub candidate: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraderPin {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub metric: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    pub rule: GateRule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_effect: Option<f64>,
    /// The difference the hypothesis expects (positive is better), at which approval
    /// computes the probability that the condition passes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_effect: Option<f64>,
    pub sided: Sided,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rater: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ReferenceMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rater: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rater_design: Option<RaterDesign>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldoutDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reuse_guard: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder_k: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder_step: Option<f64>,
}

/// Every setting the gate needs, fixed at approval from the draft or the referee's
/// configuration, and the hashes that pin the referee's state. The gate reads no
/// configuration and supplies no default for any of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolved {
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positive: Option<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Rule>,
    pub estimand: Estimand,
    pub missing_on_holdout: Policy,
    /// The configuration's handling, which the draft's validation side reads.
    pub missing: BTreeMap<MissingReason, Policy>,
    pub level: f64,
    pub per_submission_level: f64,
    pub bootstrap: u32,
    pub reference_mode: ReferenceMode,
    pub rater_design: RaterDesign,
    pub seed_derivation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder: Option<Ladder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_type: Option<OutputType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_semantics: Option<ValueSemantics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_snapshot: Option<String>,
    pub split_entry: String,
    pub seal_entry: String,
    pub holdout: String,
    pub holdout_items_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping_hash: Option<String>,
    pub holdout_access_so_far: u64,
}

/// The Ladder fixed at approval (metrics spec §11.3, D-015).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ladder {
    pub reuse_guard: String,
    pub metric: String,
    pub k: f64,
    pub step: f64,
    pub denominator: f64,
    pub psi_hat: f64,
    pub first_best: f64,
}

impl Conditions {
    /// The holdout budget B (SPEC §8.4: 5 unless the draft sets it).
    pub fn budget(&self) -> u32 {
        self.holdout.as_ref().and_then(|h| h.budget).unwrap_or(5)
    }

    /// The draft's task, or the default task.
    pub fn task(&self) -> &str {
        self.task
            .as_deref()
            .unwrap_or(evalsig_core::records::DEFAULT_TASK)
    }

    /// The conditions as JSON, as they are hashed and checked against `gate/1`.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("conditions serialize")
    }
}

/// The conditions as JSON, checked against `gate/1`; refuses the `resolved` section,
/// which only approval writes.
pub fn parse(path: &Path) -> Result<Conditions, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, &e))?;
    let toml: toml::Table = toml::from_str(&text).map_err(|e: toml::de::Error| {
        Error::invalid_conditions(format!("{}: {e}", path.display()))
    })?;
    let doc = serde_json::to_value(toml).expect("TOML converts to JSON");
    checked(&doc, &path.display().to_string())
}

/// A document that passes `check`, as conditions.
pub fn checked(doc: &Value, origin: &str) -> Result<Conditions, Error> {
    check(doc, origin)?;
    serde_json::from_value(doc.clone())
        .map_err(|e| Error::invalid_conditions(format!("{origin}: {e}")))
}

/// The checks a draft passes when it is written and again when the referee approves
/// it, since the agent can write the stored copy: no resolved section, the gate.toml
/// schema, and a threshold for every condition that compares against one.
pub fn check(doc: &Value, origin: &str) -> Result<(), Error> {
    if doc.get("resolved").is_some() {
        return Err(Error::invalid_conditions(
            "a draft has no resolved section; approval writes it",
        ));
    }
    let schema: Value = serde_json::from_str(schemas::GATE).expect("embedded schema is JSON");
    let validator = jsonschema::options()
        .build(&schema)
        .expect("embedded schema compiles");
    let mut errors: Vec<String> = validator
        .iter_errors(doc)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    for (k, c) in doc["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if c["rule"] != "pass" && !c["value"].is_number() {
            errors.push(format!(
                "/conditions/{k}: rule {} needs a numeric value",
                c["rule"]
            ));
        }
        // A difference rule's value is a margin on the difference oriented so that positive
        // is better (D-034): how far above 0 `better_by` must be, how far below 0
        // `not_worse_than` may be.
        let on_difference = c["rule"] == "better_by" || c["rule"] == "not_worse_than";
        if on_difference && c["value"].as_f64().is_some_and(|v| v < 0.0) {
            errors.push(format!(
                "/conditions/{k}: the value of {} is a margin of at least 0",
                c["rule"]
            ));
        }
        if c["min_effect"].is_number() && c["rule"] != "better_by" {
            errors.push(format!(
                "/conditions/{k}: min_effect belongs to better_by, not {}",
                c["rule"]
            ));
        }
        if c["expected_effect"].is_number() && !on_difference {
            errors.push(format!(
                "/conditions/{k}: expected_effect belongs to a difference rule, not {}",
                c["rule"]
            ));
        }
        // human_parity's value is the alternative annotator test's epsilon (D-017).
        if c["metric"] == "human_parity"
            && c["value"].as_f64().is_some_and(|e| !(e > 0.0 && e < 1.0))
        {
            errors.push(format!(
                "/conditions/{k}: the epsilon of human_parity must lie strictly between 0 and 1"
            ));
        }
        // The gate computes intervals by metrics spec §1.1 only; a named method it would
        // not use is refused rather than ignored.
        if c["interval"].as_str().is_some_and(|m| m != "auto") {
            errors.push(format!(
                "/conditions/{k}: interval {} is not available; use auto",
                c["interval"]
            ));
        }
    }
    if !errors.is_empty() {
        return Err(Error::invalid_conditions(format!(
            "{origin}: {}",
            errors.join("; ")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../examples/gate.toml");

    fn example() -> Conditions {
        let table: toml::Table = toml::from_str(EXAMPLE).unwrap();
        checked(&serde_json::to_value(table).unwrap(), "examples/gate.toml").unwrap()
    }

    fn resolved() -> Resolved {
        Resolved {
            task: "default".into(),
            positive: Some(Rule::OrdinalAtLeast(2)),
            decision: None,
            estimand: Estimand::SingleShot,
            missing_on_holdout: Policy::Worst,
            missing: BTreeMap::from([(MissingReason::Error, Policy::Worst)]),
            level: 0.95,
            per_submission_level: 0.99,
            bootstrap: 10_000,
            reference_mode: ReferenceMode::Soft,
            rater_design: RaterDesign::Auto,
            seed_derivation: "blake3(approved || candidate snapshot)".into(),
            ladder: Some(Ladder {
                reuse_guard: "ladder".into(),
                metric: "false_alarm_rate".into(),
                k: 2.0,
                step: 0.04,
                denominator: 500.0,
                psi_hat: 0.1,
                first_best: 0.2,
            }),
            output_type: Some(OutputType::Ordinal),
            levels: Some(4),
            value_semantics: Some(ValueSemantics::Decision),
            baseline_snapshot: Some(format!("blake3:{}", "a".repeat(64))),
            split_entry: format!("blake3:{}", "b".repeat(64)),
            seal_entry: format!("blake3:{}", "c".repeat(64)),
            holdout: format!("blake3:{}", "d".repeat(64)),
            holdout_items_hash: format!("blake3:{}", "e".repeat(64)),
            mapping_hash: None,
            holdout_access_so_far: 0,
        }
    }

    #[test]
    fn an_approved_document_reads_back_to_the_same_json() {
        let mut doc = example();
        doc.resolved = Some(resolved());
        let text = toml::to_string(&doc).unwrap();
        let back: Conditions = toml::from_str(&text).unwrap();
        assert_eq!(back.to_json(), doc.to_json());
        assert_eq!(back.budget(), 5);
    }

    #[test]
    fn a_resolved_setting_left_out_is_an_error_not_a_default() {
        let mut doc = example();
        doc.resolved = Some(resolved());
        let mut v = doc.to_json();
        v["resolved"].as_object_mut().unwrap().remove("bootstrap");
        let text = toml::to_string(&v).unwrap();
        let err = toml::from_str::<Conditions>(&text).unwrap_err();
        assert!(err.to_string().contains("bootstrap"), "{err}");
    }

    #[test]
    fn a_draft_with_a_resolved_section_is_refused() {
        let mut v = example().to_json();
        v["resolved"] = serde_json::to_value(resolved()).unwrap();
        let err = checked(&v, "draft").unwrap_err();
        assert_eq!(err.code, "invalid_conditions");
    }
}
