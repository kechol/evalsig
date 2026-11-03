//! `check.toml`, the subset of `gate.toml` for a non-preregistered check of one run
//! against interval-based absolute criteria (D-021).

use serde::{Deserialize, Serialize};

use crate::config::{Estimand, Reference};
use crate::rules::Rule;

/// Sections of `gate.toml` that a check refuses.
pub const GATE_ONLY: [&str; 5] = ["hypothesis", "split", "compare", "holdout", "budget"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckFile {
    pub name: Option<String>,
    pub estimand: Option<Estimand>,
    pub min_items: Option<u64>,
    pub replicates: Option<u32>,
    pub positive: Option<Rule>,
    pub decision: Option<Rule>,
    pub reference: Option<Reference>,
    pub grader: Option<GraderSpec>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraderSpec {
    pub id: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbsoluteRule {
    LowerBoundAtLeast,
    UpperBoundAtMost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sided {
    One,
    Two,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub metric: String,
    pub rule: AbsoluteRule,
    pub value: f64,
    pub sided: Sided,
    /// `auto`, the only method a check offers.
    #[serde(default)]
    pub interval: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CheckError {
    #[error("{0} belongs to gate.toml; a check has no {0}")]
    GateOnly(&'static str),
    #[error(
        "rule {0:?} needs arms or a referee; a check allows lower_bound_at_least and upper_bound_at_most"
    )]
    GateRule(String),
    #[error("interval {0:?} is not available in a check; use auto")]
    Interval(String),
    #[error("a check needs at least one condition")]
    NoConditions,
    #[error("{0}")]
    Toml(String),
}

/// Parse a check file, naming gate-only sections and rules explicitly.
pub fn parse(text: &str) -> Result<CheckFile, CheckError> {
    let raw: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| CheckError::Toml(e.to_string()))?;
    if let Some(k) = GATE_ONLY.iter().find(|k| raw.contains_key(**k)) {
        return Err(CheckError::GateOnly(k));
    }
    for c in raw
        .get("conditions")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(rule) = c.get("rule").and_then(|r| r.as_str())
            && !matches!(rule, "lower_bound_at_least" | "upper_bound_at_most")
        {
            return Err(CheckError::GateRule(rule.to_owned()));
        }
        if let Some(i) = c.get("interval").and_then(|i| i.as_str())
            && i != "auto"
        {
            return Err(CheckError::Interval(i.to_owned()));
        }
    }
    let file: CheckFile = toml::from_str(text).map_err(|e| CheckError::Toml(e.to_string()))?;
    if file.conditions.is_empty() {
        return Err(CheckError::NoConditions);
    }
    Ok(file)
}

impl Condition {
    /// The two-sided level whose interval gives this condition's bound at `level`.
    pub fn interval_level(&self, level: f64) -> f64 {
        match self.sided {
            Sided::One => 2.0 * level - 1.0,
            Sided::Two => level,
        }
    }

    /// The bound the rule reads and whether it holds; `None` without an interval.
    pub fn judge(&self, interval: Option<(f64, f64)>) -> Option<(f64, bool)> {
        let (lo, hi) = interval?;
        Some(match self.rule {
            AbsoluteRule::LowerBoundAtLeast => (lo, lo >= self.value),
            AbsoluteRule::UpperBoundAtMost => (hi, hi <= self.value),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"
name = "triage stays accurate"
min_items = 100
[[conditions]]
metric = "accuracy"
rule = "lower_bound_at_least"
value = 0.7
sided = "one"
"#;

    #[test]
    fn checks_take_the_gate_subset_only() {
        let f = parse(OK).unwrap();
        assert_eq!(f.conditions[0].interval_level(0.95), 0.8999999999999999);
        assert_eq!(
            parse(&format!("hypothesis = \"x\"\n{OK}")),
            Err(CheckError::GateOnly("hypothesis"))
        );
        assert_eq!(
            parse(&format!("{OK}[holdout]\nbudget = 5\n")),
            Err(CheckError::GateOnly("holdout"))
        );
        let delta = OK.replace("lower_bound_at_least", "not_worse_than");
        assert_eq!(
            parse(&delta),
            Err(CheckError::GateRule("not_worse_than".into()))
        );
        assert!(matches!(
            parse(&OK.replace("sided = \"one\"\n", "")),
            Err(CheckError::Toml(_))
        ));
        assert_eq!(
            parse("name = \"x\"\nconditions = []\n"),
            Err(CheckError::NoConditions)
        );
    }

    #[test]
    fn rules_read_the_matching_end_of_the_interval() {
        let c = parse(OK).unwrap().conditions.remove(0);
        assert_eq!(c.judge(Some((0.72, 0.8))), Some((0.72, true)));
        assert_eq!(c.judge(Some((0.69, 0.8))), Some((0.69, false)));
        assert_eq!(c.judge(None), None);
    }
}
