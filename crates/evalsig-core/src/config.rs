//! The project configuration file `.evalsig/config.toml` (D-004).
//!
//! Every table rejects unknown keys, so a typo never silently falls back to a default.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as Json;

use crate::missing::{MissingPolicy, Policy};
use crate::records::{MissingReason, OutputType, Value, ValueSemantics};
use crate::rules::{Rule, RuleSet};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub project: Project,
    #[serde(default)]
    pub store: Store,
    #[serde(default)]
    pub stats: Stats,
    #[serde(default)]
    pub missing: BTreeMap<MissingReason, Policy>,
    #[serde(default)]
    pub reference: Reference,
    #[serde(default)]
    pub compare: Compare,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    #[serde(default)]
    pub checks: Checks,
    #[serde(default, rename = "import", skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<Import>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positive: Option<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Rule>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tasks: BTreeMap<String, TaskRules>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRules {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positive: Option<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<Rule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentMode {
    #[default]
    Full,
    Hash,
    Redact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    #[default]
    Sqlite,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Store {
    #[serde(default)]
    pub content: ContentMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redact_patterns: Vec<String>,
    #[serde(default)]
    pub engine: Engine,
}

/// What performance is estimated (metrics spec §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Estimand {
    /// One production run.
    #[default]
    SingleShot,
    Majority(u32),
    Any(u32),
    All(u32),
}

impl fmt::Display for Estimand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Estimand::SingleShot => f.write_str("single_shot"),
            Estimand::Majority(k) => write!(f, "majority@{k}"),
            Estimand::Any(k) => write!(f, "any@{k}"),
            Estimand::All(k) => write!(f, "all@{k}"),
        }
    }
}

impl FromStr for Estimand {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "single_shot" {
            return Ok(Estimand::SingleShot);
        }
        let err =
            || format!("unknown estimand {s:?}; expected single_shot, majority@k, any@k or all@k");
        let (rule, k) = s.split_once('@').ok_or_else(err)?;
        let k: u32 = k.parse().ok().filter(|k| *k >= 1).ok_or_else(err)?;
        match rule {
            "majority" => Ok(Estimand::Majority(k)),
            "any" => Ok(Estimand::Any(k)),
            "all" => Ok(Estimand::All(k)),
            _ => Err(err()),
        }
    }
}

impl Serialize for Estimand {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Estimand {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Interval method for rates (metrics spec §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalMethod {
    #[default]
    Auto,
    Wilson,
    ClopperPearson,
    Jeffreys,
    Bootstrap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stats {
    #[serde(default = "Stats::default_level")]
    pub level: f64,
    #[serde(default = "Stats::default_bootstrap")]
    pub bootstrap: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default)]
    pub estimand: Estimand,
    #[serde(default)]
    pub interval: IntervalMethod,
}

impl Stats {
    fn default_level() -> f64 {
        0.95
    }

    fn default_bootstrap() -> u32 {
        2000
    }
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            level: Stats::default_level(),
            bootstrap: Stats::default_bootstrap(),
            seed: None,
            estimand: Estimand::default(),
            interval: IntervalMethod::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceMode {
    Single,
    Majority,
    #[default]
    Soft,
    Latent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaterDesign {
    #[default]
    Auto,
    Nested,
    Crossed,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    #[serde(default)]
    pub mode: ReferenceMode,
    #[serde(default)]
    pub rater_design: RaterDesign,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compare {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub min_effect: BTreeMap<String, f64>,
    #[serde(default = "Compare::default_validation_warn_after")]
    pub validation_warn_after: u32,
}

impl Compare {
    fn default_validation_warn_after() -> u32 {
        20
    }
}

impl Default for Compare {
    fn default() -> Self {
        Compare {
            min_effect: BTreeMap::new(),
            validation_warn_after: Compare::default_validation_warn_after(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cost {
    /// Rows: reference negative, positive. Columns: judgment levels.
    pub matrix: Vec<Vec<f64>>,
    pub prevalence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio_range: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checks {
    #[serde(default = "yes")]
    pub fail_on_mixed_grader_versions: bool,
    #[serde(default = "yes")]
    pub fail_on_unexplained_missing: bool,
    #[serde(default = "yes")]
    pub warn_missing_rate_differs: bool,
    #[serde(default = "yes")]
    pub warn_identical_replicates: bool,
    #[serde(default = "yes")]
    pub warn_run_variance_exceeds_delta: bool,
}

fn yes() -> bool {
    true
}

impl Default for Checks {
    fn default() -> Self {
        Checks {
            fail_on_mixed_grader_versions: true,
            fail_on_unexplained_missing: true,
            warn_missing_rate_differs: true,
            warn_identical_replicates: true,
            warn_run_variance_exceeds_delta: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Adapter {
    Jsonl,
    Csv,
    Inspect,
    Promptfoo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportAs {
    #[default]
    Judgment,
    Label,
}

/// One `[[import]]` block: how a flat source maps to records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub name: String,
    pub adapter: Adapter,
    pub path: String,
    #[serde(default, rename = "as")]
    pub import_as: ImportAs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_semantics: Option<ValueSemantics>,
    /// For harness adapters: which score becomes the judgment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<String>,
    #[serde(default)]
    pub map: FieldMap,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub value_map: BTreeMap<String, Value>,
}

/// Where each record field comes from: a path into the source row (`$.a.b` for
/// JSON, a column name for CSV) or, for some fields, a constant.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldMap {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rater_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gen_replicate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade_replicate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_type: Option<OutputType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub strata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::records::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labeled_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    Parse(String),
    #[error("{field}: {message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
}

impl Config {
    pub fn from_toml(text: &str) -> Result<Config, ConfigError> {
        let config: Config = toml::from_str(text).map_err(|e| ConfigError::Parse(hint(e)))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |field, message: &str| {
            Err(ConfigError::Invalid {
                field,
                message: message.to_owned(),
            })
        };
        if !(self.stats.level > 0.0 && self.stats.level < 1.0) {
            return invalid("stats.level", "must lie strictly between 0 and 1");
        }
        if self.stats.bootstrap == 0 {
            return invalid("stats.bootstrap", "must be at least 1");
        }
        if self
            .compare
            .min_effect
            .values()
            .any(|v| !(v.is_finite() && *v >= 0.0))
        {
            return invalid(
                "compare.min_effect",
                "values must be finite and non-negative",
            );
        }
        if let Some(cost) = &self.cost {
            if cost.matrix.len() != 2 {
                return invalid(
                    "cost.matrix",
                    "needs two rows (reference negative, positive)",
                );
            }
            let width = cost.matrix[0].len();
            if width < 2 || cost.matrix[1].len() != width {
                return invalid("cost.matrix", "rows must have the same length, at least 2");
            }
            if cost.matrix.iter().flatten().any(|c| !c.is_finite()) {
                return invalid("cost.matrix", "costs must be finite");
            }
            if !(cost.prevalence > 0.0 && cost.prevalence < 1.0) {
                return invalid("cost.prevalence", "must lie strictly between 0 and 1");
            }
            if let Some([lo, hi]) = cost.ratio_range
                && !(lo > 0.0 && lo <= hi && hi.is_finite())
            {
                return invalid(
                    "cost.ratio_range",
                    "must be [low, high] with 0 < low <= high",
                );
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for import in &self.imports {
            if !names.insert(import.name.as_str()) {
                return invalid(
                    "import.name",
                    &format!("duplicate import name {:?}", import.name),
                );
            }
            if let Some(l) = import.map.levels
                && l < 2
            {
                return invalid("import.map.levels", "must be at least 2");
            }
        }
        Ok(())
    }

    /// Positive definitions, project-wide and per task.
    pub fn positive(&self) -> RuleSet {
        self.rule_set(|t| t.positive, self.project.positive)
    }

    /// Decision rules, project-wide and per task.
    pub fn decision(&self) -> RuleSet {
        self.rule_set(|t| t.decision, self.project.decision)
    }

    fn rule_set(
        &self,
        pick: impl Fn(&TaskRules) -> Option<Rule>,
        default: Option<Rule>,
    ) -> RuleSet {
        RuleSet {
            default,
            per_task: self
                .project
                .tasks
                .iter()
                .filter_map(|(name, t)| pick(t).map(|r| (name.clone(), r)))
                .collect(),
        }
    }

    pub fn missing_policy(&self) -> MissingPolicy {
        self.missing
            .iter()
            .fold(MissingPolicy::default(), |p, (r, policy)| {
                p.with_override(*r, *policy)
            })
    }

    /// The configuration as JSON with every default filled in, so that two files
    /// meaning the same thing hash the same once canonicalized.
    pub fn to_json(&self) -> Json {
        serde_json::to_value(self).expect("configuration types serialize to JSON")
    }
}

/// Add a "did you mean" to unknown-key errors when a known key is one edit away.
fn hint(e: toml::de::Error) -> String {
    let msg = e.to_string();
    let Some(rest) = msg.split("unknown field `").nth(1) else {
        return msg;
    };
    let Some((unknown, expected)) = rest.split_once('`') else {
        return msg;
    };
    let suggestion = expected
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|k| edit_distance(unknown, k) <= 2)
        .min_by_key(|k| edit_distance(unknown, k));
    match suggestion {
        Some(k) => format!("{msg}\nhint: did you mean `{k}`?"),
        None => msg,
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(ca != *cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../examples/evalsig.toml");

    #[test]
    fn the_example_parses() {
        let c = Config::from_toml(EXAMPLE).unwrap();
        assert_eq!(c.project.name.as_deref(), Some("support-ticket-triage"));
        assert_eq!(
            c.positive().for_task("urgency"),
            Some(&Rule::OrdinalAtLeast(3))
        );
        assert_eq!(
            c.positive().for_task("other"),
            Some(&Rule::OrdinalAtLeast(2))
        );
        assert_eq!(
            c.decision().for_task("urgency"),
            Some(&Rule::OrdinalAtLeast(2))
        );
        assert_eq!(c.store.content, ContentMode::Full);
        assert_eq!(c.stats.seed, Some(20261029));
        assert_eq!(c.stats.estimand, Estimand::SingleShot);
        assert_eq!(
            c.missing_policy().policy(MissingReason::Refused),
            Policy::Worst
        );
        assert_eq!(
            c.missing_policy().policy(MissingReason::NotRun),
            Policy::Drop
        );
        assert_eq!(c.imports.len(), 3);
        assert_eq!(c.imports[1].import_as, ImportAs::Label);
        assert_eq!(c.imports[1].value_map["yes"], Value::Bool(true));
        assert_eq!(c.imports[2].value_map["P"], Value::Float(0.5));
        assert_eq!(c.imports[0].map.strata["lang"], "$.meta.lang");
    }

    #[test]
    fn unknown_keys_are_rejected_with_a_hint() {
        let err = Config::from_toml("[stats]\nlevle = 0.9\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field `levle`"), "{err}");
        assert!(err.contains("did you mean `level`?"), "{err}");
        let err = Config::from_toml("[project]\nnmae = \"x\"\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("did you mean `name`?"), "{err}");
        let err = Config::from_toml("[unknown_section]\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field `unknown_section`"), "{err}");
        let err = Config::from_toml("[missing]\nnap = \"drop\"\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("nap"), "{err}");
        assert!(Config::from_toml("[project]\npositive = { ordinal_atleast = 2 }\n").is_err());
    }

    #[test]
    fn values_are_validated() {
        for bad in [
            "[stats]\nlevel = 1.5\n",
            "[stats]\nbootstrap = 0\n",
            "[stats]\nestimand = \"majority@0\"\n",
            "[stats]\nestimand = \"best@3\"\n",
            "[cost]\nmatrix = [[0, 1]]\nprevalence = 0.1\n",
            "[cost]\nmatrix = [[0, 1], [1]]\nprevalence = 0.1\n",
            "[cost]\nmatrix = [[0, 1], [1, 0]]\nprevalence = 1.0\n",
            "[cost]\nmatrix = [[0, 1], [1, 0]]\nprevalence = 0.1\nratio_range = [10, 3]\n",
            "[compare]\nmin_effect = { accuracy = -0.1 }\n",
            "[[import]]\nname = \"a\"\nadapter = \"csv\"\npath = \"x\"\n[[import]]\nname = \"a\"\nadapter = \"csv\"\npath = \"y\"\n",
        ] {
            assert!(Config::from_toml(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn estimands_round_trip() {
        for s in ["single_shot", "majority@3", "any@5", "all@2"] {
            assert_eq!(s.parse::<Estimand>().unwrap().to_string(), s);
        }
    }

    #[test]
    fn json_form_fills_defaults() {
        let empty = Config::from_toml("").unwrap().to_json();
        let explicit = Config::from_toml(
            "[stats]\nlevel = 0.95\nbootstrap = 2000\n[store]\ncontent = \"full\"\n",
        )
        .unwrap()
        .to_json();
        assert_eq!(empty, explicit);
        assert_eq!(empty["stats"]["level"], 0.95);
        assert_eq!(empty["missing"], serde_json::json!({}));
        let example = Config::from_toml(EXAMPLE).unwrap().to_json();
        assert_eq!(
            example["project"]["positive"],
            serde_json::json!({"ordinal_at_least": 2})
        );
        assert_eq!(example["stats"]["estimand"], "single_shot");
    }
}
