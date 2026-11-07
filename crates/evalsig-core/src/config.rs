//! The project configuration file `.evalsig/config.toml` (D-004).
//!
//! Every table rejects unknown keys, so a typo never silently falls back to a default.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as Json;

use crate::missing::{MissingPolicy, Policy};
use crate::records::{MissingReason, OutputType, Source, Value, ValueSemantics};
use crate::rules::{Rule, RuleSet};

/// The whole project configuration; every table is optional.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Project name and the positive definition and decision rule.
    pub project: Project,
    /// How content is stored.
    pub store: Store,
    /// Interval level, resampling, estimand and interval method.
    pub stats: Stats,
    /// Missing policy per missing reason; unlisted reasons keep their default.
    pub missing: BTreeMap<MissingReason, Policy>,
    /// How several human labels become a reference.
    pub reference: Reference,
    /// Settings for comparing two arms.
    pub compare: Compare,
    /// Settings for agreement with people.
    pub agree: Agree,
    /// Cost matrix for expected cost; absent when no costs are configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    /// Switches for the validity checks.
    pub checks: Checks,
    /// The `[[import]]` blocks, in file order.
    #[serde(rename = "import", skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<Import>,
    /// How the dashboard shows its pages. It changes no result, so it stays out of the
    /// stored configuration that provenance names.
    #[serde(skip_serializing)]
    pub dashboard: Dashboard,
}

/// The dashboard's display settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Dashboard {
    /// The language of the pages' own text (D-033).
    pub language: Language,
}

/// A language the dashboard's own text is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// English, the default.
    #[default]
    En,
    /// Japanese.
    Ja,
}

/// Project-wide name and the label-side and judgment-side rules.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Project {
    /// Display name of the project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Positive definition: which reference label values count as positive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positive: Option<Rule>,
    /// Decision rule: from which judgment value the system says positive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<Rule>,
    /// Per-task overrides of `positive` and `decision`, keyed by task name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tasks: BTreeMap<String, TaskRules>,
}

/// Per-task overrides of the project's positive definition and decision rule.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TaskRules {
    /// Positive definition for this task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positive: Option<Rule>,
    /// Decision rule for this task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<Rule>,
}

/// How the text of inputs, outputs and grader rationales is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentMode {
    /// Keep the text in full (default).
    #[default]
    Full,
    /// Keep only the hash of the text.
    Hash,
    /// Keep the text with `redact_patterns` matches replaced by `[redacted]`.
    Redact,
}

/// Content storage settings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Store {
    /// How content is stored; default `full`.
    pub content: ContentMode,
    /// Regular expressions whose matches are replaced by `[redacted]` in redact mode.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redact_patterns: Vec<String>,
}

/// What performance is estimated (metrics spec §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Estimand {
    /// One production run.
    #[default]
    SingleShot,
    /// Majority of `k` generation replicates is positive; `k` is at least 1.
    Majority(u32),
    /// Any of `k` generation replicates is positive; `k` is at least 1.
    Any(u32),
    /// All `k` generation replicates are positive; `k` is at least 1.
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
    /// Choose a method from the data (default).
    #[default]
    Auto,
    /// Wilson score interval.
    Wilson,
    /// Clopper-Pearson exact interval.
    ClopperPearson,
    /// Jeffreys interval.
    Jeffreys,
    /// Bootstrap percentile interval.
    Bootstrap,
}

/// Interval and resampling settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stats {
    /// Confidence level of intervals, strictly between 0 and 1; default 0.95.
    pub level: f64,
    /// Number of bootstrap resamples; default 2000.
    pub bootstrap: u32,
    /// Seed for resampling; absent means the caller picks one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Which performance is estimated; default `single_shot`.
    pub estimand: Estimand,
    /// Interval method for rates; default `auto`.
    pub interval: IntervalMethod,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            level: 0.95,
            bootstrap: 2000,
            seed: None,
            estimand: Estimand::default(),
            interval: IntervalMethod::default(),
        }
    }
}

/// How several human labels of an item become one reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceMode {
    /// One rater or an adjudicated label.
    Single,
    /// The majority of the raters.
    Majority,
    /// The share of raters saying positive (default).
    #[default]
    Soft,
    /// A latent class model (Dawid-Skene); needs three or more raters per item.
    Latent,
}

/// Whether the same raters label many items or different raters label each item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaterDesign {
    /// Detect the design from the rater overlap (default).
    #[default]
    Auto,
    /// Different raters label each item.
    Nested,
    /// The same few raters label many items.
    Crossed,
}

/// Reference construction settings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Reference {
    /// Reference mode for items with two or more human raters; default `soft`.
    pub mode: ReferenceMode,
    /// Rater design; default `auto`.
    pub rater_design: RaterDesign,
}

/// Settings for comparing two arms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Compare {
    /// Minimum effect per metric name; smaller differences are not called improvements.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub min_effect: BTreeMap<String, f64>,
    /// Warn that the validation split is due for a rebuild after this many comparisons.
    pub validation_warn_after: u32,
}

impl Default for Compare {
    fn default() -> Self {
        Compare {
            min_effect: BTreeMap::new(),
            validation_warn_after: 20,
        }
    }
}

/// Settings for agreement with people.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agree {
    /// How far the system may trail a human rater in the alternative annotator test
    /// (D-017).
    pub parity_epsilon: f64,
}

impl Default for Agree {
    fn default() -> Self {
        Agree {
            parity_epsilon: crate::parity::EPSILON,
        }
    }
}

/// Cost matrix and prevalence for expected cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cost {
    /// Rows: reference negative, positive. Columns: judgment levels.
    pub matrix: Vec<Vec<f64>>,
    /// Production prevalence of positives, not the test-set share.
    pub prevalence: f64,
    /// Range of C_FN / C_FP for the cost-ratio sensitivity analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio_range: Option<[f64; 2]>,
}

impl Cost {
    /// C_FN / C_FP read off the matrix's corners: a positive judged at the lowest
    /// level against a negative judged at the highest. `None` when either is not a
    /// positive number.
    pub fn corner_ratio(&self) -> Option<f64> {
        let fp = *self.matrix.first()?.last()?;
        let fn_ = *self.matrix.get(1)?.first()?;
        (fp > 0.0 && fn_ > 0.0).then(|| fn_ / fp)
    }
}

/// Switches for the validity checks; all default to true.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Checks {
    /// Fail when the compared runs mix grader versions.
    pub fail_on_mixed_grader_versions: bool,
    /// Fail when missing judgments have no explanation.
    pub fail_on_unexplained_missing: bool,
    /// Warn when the missing rate differs between arms.
    pub warn_missing_rate_differs: bool,
    /// Warn when all generation replicates of an item have the same output.
    pub warn_identical_replicates: bool,
    /// Warn when run-to-run variance exceeds the difference being judged.
    pub warn_run_variance_exceeds_delta: bool,
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

/// The source format of an import block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Adapter {
    /// One JSON object per line.
    Jsonl,
    /// Comma-separated rows with a header.
    Csv,
    /// Inspect logs (`.eval` or `.json`).
    Inspect,
    /// promptfoo results.
    Promptfoo,
}

/// What each source row becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportAs {
    /// One judgment per row (default).
    #[default]
    Judgment,
    /// One reference label per row.
    Label,
}

/// One `[[import]]` block: how a flat source maps to records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    /// Name of the import block.
    pub name: String,
    /// Source format.
    pub adapter: Adapter,
    /// Path or glob of the source files.
    pub path: String,
    /// Whether rows become judgments or reference labels; default judgment (`as`).
    #[serde(default, rename = "as")]
    pub import_as: ImportAs,
    /// Value semantics of the imported judgments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_semantics: Option<ValueSemantics>,
    /// For harness adapters: which score becomes the judgment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<String>,
    /// Where each record field comes from.
    #[serde(default)]
    pub map: FieldMap,
    /// Maps source values to record values; unmappable values become missing.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub value_map: BTreeMap<String, Value>,
}

impl Import {
    /// `value_map` with JSON values, as mappings and adapters read it.
    pub fn value_map_json(&self) -> BTreeMap<String, serde_json::Value> {
        self.value_map
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    serde_json::to_value(v).expect("values serialize"),
                )
            })
            .collect()
    }
}

/// Where each record field comes from: a path into the source row (`$.a.b` for
/// JSON, a column name for CSV) or, for some fields, a constant.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FieldMap {
    /// Source of the item ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    /// Source of the run ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Source of the rater ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rater_id: Option<String>,
    /// Source of the task name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Source of the generation replicate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gen_replicate: Option<String>,
    /// Source of the grading replicate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_replicate: Option<String>,
    /// Source of the judged or labelled value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Output type of the values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_type: Option<OutputType>,
    /// Number of ordinal levels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
    /// Subtracted from source values before use, for sources whose levels start at 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    /// Stratum name to source, one entry per stratum.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub strata: BTreeMap<String, String>,
    /// Source of the system-independent input reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_ref: Option<String>,
    /// Who produced the labels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// Source of the labelling timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labeled_at: Option<String>,
    /// Version of the label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
}

/// Why a configuration text was rejected.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The TOML could not be parsed or has an unknown key or a bad value.
    #[error("{0}")]
    Parse(String),
    /// A value parses but violates a constraint.
    #[error("{field}: {message}")]
    Invalid {
        /// The offending configuration key.
        field: &'static str,
        /// What is wrong with it.
        message: String,
    },
}

impl Config {
    /// Parses and validates a configuration file's text.
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
        let rules = std::iter::once((self.project.positive, self.project.decision))
            .chain(
                self.project
                    .tasks
                    .values()
                    .map(|t| (t.positive, t.decision)),
            )
            .flat_map(|(p, d)| [p, d])
            .flatten();
        for r in rules {
            if matches!(r, Rule::ScoreAtLeast(t) if !t.is_finite())
                || matches!(r, Rule::Equals(Value::Float(t)) if !t.is_finite())
            {
                return invalid("project", "rule thresholds must be finite");
            }
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
        if !(self.agree.parity_epsilon > 0.0 && self.agree.parity_epsilon < 1.0) {
            return invalid("agree.parity_epsilon", "must lie strictly between 0 and 1");
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
            if import.map.version == Some(0) {
                return invalid("import.map.version", "must be at least 1");
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

    /// The missing policy with this configuration's per-reason overrides applied.
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

/// Add a "did you mean" to unknown-key errors when a known key is within two edits.
/// It reads serde's "unknown field `x`, expected one of `a`, `b`" wording; the tests
/// pin that wording so a toml upgrade that changes it fails loudly.
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
    fn the_dashboard_language_stays_out_of_the_stored_configuration() {
        let ja = Config::from_toml("[dashboard]\nlanguage = \"ja\"\n").unwrap();
        assert_eq!(ja.dashboard.language, Language::Ja);
        assert_eq!(Config::default().dashboard.language, Language::En);
        // Display only: the configuration that provenance hashes is the same.
        assert_eq!(ja.to_json(), Config::default().to_json());
        assert!(Config::from_toml("[dashboard]\nlanguage = \"fr\"\n").is_err());
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
            "[project]\npositive = { score_at_least = nan }\n",
            "[[import]]\nname = \"a\"\nadapter = \"csv\"\npath = \"x\"\nmap.version = 0\n",
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
