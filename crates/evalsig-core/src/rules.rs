//! Positive definitions (which reference label values count as positive) and decision
//! rules (from which judgment value the system counts as saying positive). The two are
//! configured separately and may differ per task.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::records::Value;

/// A rule that classifies a value as positive. Unordered classes are handled one class
/// against the rest with `equals`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Rule {
    /// Ordinal levels at or above this level are positive.
    OrdinalAtLeast(i64),
    /// Scores at or above this threshold are positive.
    ScoreAtLeast(f64),
    /// Exactly this value is positive (binary `true`, or one class of several).
    /// `boolean = true` is the same rule, as gate.toml writes it.
    #[serde(alias = "boolean")]
    Equals(Value),
}

impl Rule {
    /// `None` when the rule does not apply to the value's type.
    pub fn is_positive(&self, value: Value) -> Option<bool> {
        match (*self, value) {
            (Rule::OrdinalAtLeast(t), Value::Int(v)) => Some(v >= t),
            (Rule::ScoreAtLeast(t), Value::Int(_) | Value::Float(_)) => Some(value.as_f64() >= t),
            (Rule::Equals(Value::Bool(t)), Value::Bool(v)) => Some(v == t),
            (Rule::Equals(Value::Int(t)), Value::Int(v)) => Some(v == t),
            (Rule::Equals(Value::Float(t)), Value::Float(v)) => Some(v == t),
            _ => None,
        }
    }
}

/// A project-level rule with optional per-task overrides.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RuleSet {
    /// The rule for tasks without an override.
    pub default: Option<Rule>,
    /// The rule per task name.
    pub per_task: BTreeMap<String, Rule>,
}

impl RuleSet {
    /// A rule set with this default and no overrides.
    pub fn new(default: Rule) -> Self {
        RuleSet {
            default: Some(default),
            per_task: BTreeMap::new(),
        }
    }

    /// Set the rule for one task.
    pub fn with_task(mut self, task: &str, rule: Rule) -> Self {
        self.per_task.insert(task.to_owned(), rule);
        self
    }

    /// The rule for `task`; the project default when the task has no override.
    pub fn for_task(&self, task: &str) -> Option<&Rule> {
        self.per_task.get(task).or(self.default.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::DEFAULT_TASK;

    #[test]
    fn rules_classify_values_of_their_type() {
        let r = Rule::OrdinalAtLeast(2);
        assert_eq!(r.is_positive(Value::Int(1)), Some(false));
        assert_eq!(r.is_positive(Value::Int(2)), Some(true));
        assert_eq!(r.is_positive(Value::Bool(true)), None);
        let s = Rule::ScoreAtLeast(0.5);
        assert_eq!(s.is_positive(Value::Float(0.49)), Some(false));
        assert_eq!(s.is_positive(Value::Float(0.5)), Some(true));
        assert_eq!(s.is_positive(Value::Int(1)), Some(true));
        let b = Rule::Equals(Value::Bool(true));
        assert_eq!(b.is_positive(Value::Bool(false)), Some(false));
        // One class against the rest.
        let c = Rule::Equals(Value::Int(3));
        assert_eq!(c.is_positive(Value::Int(3)), Some(true));
        assert_eq!(c.is_positive(Value::Int(0)), Some(false));
    }

    #[test]
    fn per_task_rules_override_the_project_rule() {
        let set =
            RuleSet::new(Rule::OrdinalAtLeast(2)).with_task("severity", Rule::OrdinalAtLeast(3));
        assert_eq!(set.for_task("severity"), Some(&Rule::OrdinalAtLeast(3)));
        assert_eq!(set.for_task("other"), Some(&Rule::OrdinalAtLeast(2)));
        assert_eq!(set.for_task(DEFAULT_TASK), Some(&Rule::OrdinalAtLeast(2)));
        assert_eq!(RuleSet::default().for_task("x"), None);
    }

    #[test]
    fn rules_read_from_one_key_tables() {
        let rule = |s: &str| {
            toml::from_str::<std::collections::BTreeMap<String, Rule>>(&format!("r = {s}"))
        };
        assert_eq!(
            rule("{ ordinal_at_least = 2 }").unwrap()["r"],
            Rule::OrdinalAtLeast(2)
        );
        assert_eq!(
            rule("{ score_at_least = 0.5 }").unwrap()["r"],
            Rule::ScoreAtLeast(0.5)
        );
        assert_eq!(
            rule("{ equals = true }").unwrap()["r"],
            Rule::Equals(Value::Bool(true))
        );
        assert!(rule("{ ordinal_at_most = 2 }").is_err());
    }
}
