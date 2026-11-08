//! `check.toml`, the subset of `gate.toml` for a non-preregistered check of one run
//! against interval-based absolute criteria (D-021).

use serde::Deserialize;

use crate::config::{Estimand, Reference};
use crate::gate::{self, Evidence, Sided};
use crate::rules::Rule;

/// Sections of `gate.toml` that a check refuses.
pub const GATE_ONLY: [&str; 5] = ["hypothesis", "split", "compare", "holdout", "budget"];

/// A `check.toml` file: absolute-threshold conditions on one run, without arms or a
/// referee.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckFile {
    /// Name of the check.
    pub name: Option<String>,
    /// Which performance is estimated.
    pub estimand: Option<Estimand>,
    /// Minimum number of items the check requires.
    pub min_items: Option<u64>,
    /// Number of generation replicates the check uses.
    pub replicates: Option<u32>,
    /// Rule that decides which reference values are positive.
    pub positive: Option<Rule>,
    /// Decision rule: from which judgment value a judgment counts as positive.
    pub decision: Option<Rule>,
    /// How several human labels become a reference.
    pub reference: Option<Reference>,
    /// The grader whose judgments the check is about.
    pub grader: Option<GraderSpec>,
    /// The conditions the run must satisfy; all must hold.
    pub conditions: Vec<Condition>,
}

/// The grader a check applies to.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraderSpec {
    /// Identifier of the grader.
    pub id: String,
    /// Version of the grader; any version when omitted.
    pub version: Option<String>,
}

/// One condition of a check: a rule applied to a metric.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// Name of the metric the condition reads.
    pub metric: String,
    /// The rule applied to the metric's interval: `lower_bound_at_least` or
    /// `upper_bound_at_most`, the gate's absolute rules (`parse` refuses the others).
    pub rule: gate::Rule,
    /// The threshold the bound is compared with.
    pub value: f64,
    /// Whether the interval is one-sided or two-sided.
    pub sided: Sided,
    /// `auto`, the only method a check offers.
    #[serde(default)]
    pub interval: Option<String>,
    /// For `rate_at_least`: the score threshold t.
    #[serde(default)]
    pub threshold: Option<f64>,
    /// This condition's confidence level, instead of the configured one.
    #[serde(default)]
    pub level: Option<f64>,
}

/// Why a check file is refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CheckError {
    /// The file contains a section that only `gate.toml` has.
    #[error("{0} belongs to gate.toml; a check has no {0}")]
    GateOnly(&'static str),
    /// The rule needs arms or a referee, which a check does not have.
    #[error(
        "rule {0:?} needs arms or a referee; a check allows lower_bound_at_least and upper_bound_at_most"
    )]
    GateRule(String),
    /// `min_effect` was set, but a check's rules are absolute.
    #[error("min_effect applies to gate's difference rules; a check's rules are absolute")]
    MinEffect,
    /// The interval method is not `auto`.
    #[error("interval {0:?} is not available in a check; use auto")]
    Interval(String),
    /// The file has no condition.
    #[error("a check needs at least one condition")]
    NoConditions,
    /// The file is not valid TOML or does not match the schema.
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
        if c.get("min_effect").is_some() {
            return Err(CheckError::MinEffect);
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
    /// The two-sided level whose interval gives this condition's bound, at its own
    /// level or else the configured `level`.
    pub fn interval_level(&self, level: f64) -> f64 {
        gate::interval_level(self.level.unwrap_or(level), 1, self.sided)
    }

    /// The bound the rule reads and whether it holds, as a gate would judge it; `None`
    /// without a value and its interval.
    pub fn judge(&self, point: Option<f64>, interval: Option<(f64, f64)>) -> Option<(f64, bool)> {
        let (point, (lo, hi)) = (point?, interval?);
        let bound = if self.rule == gate::Rule::UpperBoundAtMost {
            hi
        } else {
            lo
        };
        let passed = gate::judge(
            self.rule,
            self.value,
            None,
            Some(Evidence::Value { point, lo, hi }),
        )?;
        Some((bound, passed))
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
        assert_eq!(
            parse(&format!("{OK}min_effect = 0.02\n")),
            Err(CheckError::MinEffect)
        );
        let own = parse(&format!("{OK}level = 0.99\nthreshold = 0.5\n")).unwrap();
        assert_eq!(own.conditions[0].threshold, Some(0.5));
        assert!((own.conditions[0].interval_level(0.95) - 0.98).abs() < 1e-12);
    }

    #[test]
    fn rules_read_the_matching_end_of_the_interval() {
        let c = parse(OK).unwrap().conditions.remove(0);
        assert_eq!(c.judge(Some(0.76), Some((0.72, 0.8))), Some((0.72, true)));
        assert_eq!(c.judge(Some(0.74), Some((0.69, 0.8))), Some((0.69, false)));
        assert_eq!(c.judge(Some(0.76), None), None);
    }
}
