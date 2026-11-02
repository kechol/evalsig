//! The canonical record format `records/1` (D-003).
//!
//! Optional fields stay `None` when absent so that a record serializes back to what
//! was read; defaults from the schema are applied by accessor methods instead.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

/// The schema version written by this crate.
pub const RECORDS_VERSION: &str = "records/1";

/// Default task name when a record carries none.
pub const DEFAULT_TASK: &str = "default";

/// Content-addressed reference such as `blake3:<64 hex>` or `sha256:<64 hex>`.
pub type HashRef = String;

/// RFC 3339 timestamp, kept as written.
pub type Timestamp = String;

/// One line of a `records/1` JSONL file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum Record {
    Item(Item),
    Label(Label),
    Judgment(Judgment),
    Preference(Preference),
    Run(Run),
    Content(Content),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strata: Option<Map<String, Json>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_inclusion_prob: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_ref: Option<HashRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Json>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_ref: Option<HashRef>,
}

impl Item {
    pub fn weight(&self) -> f64 {
        self.weight.unwrap_or(1.0)
    }

    /// The resampling and splitting unit: the cluster, or the item itself.
    pub fn cluster_key(&self) -> &str {
        self.cluster.as_deref().unwrap_or(&self.item_id)
    }
}

/// Who produced a label or a preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Human,
    Model,
    Consensus,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    #[default]
    Independent,
    Revision,
    Adjudication,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub item_id: String,
    pub rater_id: String,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<LabelKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_replicate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labeled_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
}

impl Label {
    pub fn task(&self) -> &str {
        self.task.as_deref().unwrap_or(DEFAULT_TASK)
    }

    pub fn kind(&self) -> LabelKind {
        self.kind.unwrap_or_default()
    }

    pub fn label_replicate(&self) -> u32 {
        self.label_replicate.unwrap_or(0)
    }

    pub fn observation(&self) -> Result<Observation<'_, Value>, RecordError> {
        Observation::from_parts(self.value.as_ref(), self.missing.as_ref())
    }
}

/// A feature (rule, aspect, assertion) that fired inside a judgment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub feature_id: String,
    pub fired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Judgment {
    pub item_id: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub gen_replicate: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade_replicate: Option<u32>,
    pub output_type: OutputType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<Feature>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<HashRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale_ref: Option<HashRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judged_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Map<String, Json>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

impl Judgment {
    pub fn task(&self) -> &str {
        self.task.as_deref().unwrap_or(DEFAULT_TASK)
    }

    pub fn grade_replicate(&self) -> u32 {
        self.grade_replicate.unwrap_or(0)
    }

    pub fn observation(&self) -> Result<Observation<'_, Value>, RecordError> {
        Observation::from_parts(self.value.as_ref(), self.missing.as_ref())
    }
}

/// Outcome of a pairwise comparison, by presentation side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceOutcome {
    Left,
    Right,
    Tie,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preference {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub left_run_id: String,
    pub right_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_gen_replicate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_gen_replicate: Option<u32>,
    pub rater_id: String,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade_replicate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<PreferenceOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale_ref: Option<HashRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judged_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

impl Preference {
    pub fn task(&self) -> &str {
        self.task.as_deref().unwrap_or(DEFAULT_TASK)
    }

    pub fn observation(&self) -> Result<Observation<'_, PreferenceOutcome>, RecordError> {
        Observation::from_parts(self.outcome.as_ref(), self.missing.as_ref())
    }
}

/// How a run's judgments are read (metrics spec §0.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueSemantics {
    /// A system decision, compared with reference labels.
    #[default]
    Decision,
    /// A grade of an output that is itself the result; no reference label needed.
    Score,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputType {
    Binary,
    Ordinal,
    Score,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemRef {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<HashRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraderKind {
    Model,
    Rule,
    Human,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GraderRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<GraderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<HashRef>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HarnessRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub run_id: String,
    pub system: SystemRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grader: Option<GraderRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Json>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_semantics: Option<ValueSemantics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_type: Option<OutputType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
}

impl Run {
    pub fn value_semantics(&self) -> ValueSemantics {
        self.value_semantics.unwrap_or_default()
    }
}

/// How much of a content blob is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stored {
    Full,
    Hash,
    Redact,
    Purged,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    #[serde(rename = "ref")]
    pub content_ref: HashRef,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored: Option<Stored>,
}

/// A judgment or label value. Integers and floats are kept apart so that a record
/// serializes back exactly as read.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
}

impl Value {
    /// The value as a number (`true` = 1, `false` = 0).
    pub fn as_f64(self) -> f64 {
        match self {
            Value::Bool(b) => f64::from(u8::from(b)),
            Value::Int(i) => i as f64,
            Value::Float(f) => f,
        }
    }
}

/// Why a judgment, label or preference has no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingReason {
    Error,
    Timeout,
    Refused,
    InvalidOutput,
    Limit,
    GraderFailed,
    NotRun,
    Invalidated,
    Excluded,
    Other,
}

impl MissingReason {
    pub const ALL: [MissingReason; 10] = [
        MissingReason::Error,
        MissingReason::Timeout,
        MissingReason::Refused,
        MissingReason::InvalidOutput,
        MissingReason::Limit,
        MissingReason::GraderFailed,
        MissingReason::NotRun,
        MissingReason::Invalidated,
        MissingReason::Excluded,
        MissingReason::Other,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Missing {
    pub reason: MissingReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Either a value or a missing marker; a record carries exactly one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Observation<'a, V> {
    Value(&'a V),
    Missing(&'a Missing),
}

impl<'a, V> Observation<'a, V> {
    fn from_parts(value: Option<&'a V>, missing: Option<&'a Missing>) -> Result<Self, RecordError> {
        match (value, missing) {
            (Some(v), None) => Ok(Observation::Value(v)),
            (None, Some(m)) => Ok(Observation::Missing(m)),
            (Some(_), Some(_)) => Err(RecordError::ValueAndMissing),
            (None, None) => Err(RecordError::NeitherValueNorMissing),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("invalid record: {0}")]
    Json(#[from] serde_json::Error),
    #[error("record has both `value` and `missing`")]
    ValueAndMissing,
    #[error("record has neither `value` nor `missing`")]
    NeitherValueNorMissing,
    #[error("value {value:?} does not match output type {output_type:?}")]
    ValueType {
        output_type: OutputType,
        value: Value,
    },
    #[error("ordinal record requires `levels`")]
    MissingLevels,
    #[error("ordinal value {value} is outside 0..{levels}")]
    LevelOutOfRange { value: i64, levels: u32 },
    #[error("`levels` must be at least 2")]
    TooFewLevels,
    #[error("score value must be finite")]
    NonFinite,
}

impl Record {
    /// Parse one JSONL line and check the rules the type system does not express.
    pub fn from_json(line: &str) -> Result<Record, RecordError> {
        let record: Record = serde_json::from_str(line)?;
        record.check()?;
        Ok(record)
    }

    /// Check value-xor-missing and that values match the declared output type.
    pub fn check(&self) -> Result<(), RecordError> {
        match self {
            Record::Judgment(j) => {
                if let Observation::Value(v) = j.observation()? {
                    check_value(j.output_type, j.levels, *v)?;
                }
                check_levels(j.levels)
            }
            Record::Label(l) => {
                l.observation()?;
                check_levels(l.levels)
            }
            Record::Preference(p) => p.observation().map(|_| ()),
            Record::Run(r) => check_levels(r.levels),
            Record::Item(_) | Record::Content(_) => Ok(()),
        }
    }
}

fn check_levels(levels: Option<u32>) -> Result<(), RecordError> {
    match levels {
        Some(l) if l < 2 => Err(RecordError::TooFewLevels),
        _ => Ok(()),
    }
}

fn check_value(
    output_type: OutputType,
    levels: Option<u32>,
    value: Value,
) -> Result<(), RecordError> {
    let mismatch = || RecordError::ValueType { output_type, value };
    match (output_type, value) {
        (OutputType::Binary, Value::Bool(_)) => Ok(()),
        (OutputType::Ordinal, Value::Int(i)) => {
            let levels = levels.ok_or(RecordError::MissingLevels)?;
            if i < 0 || i >= i64::from(levels) {
                Err(RecordError::LevelOutOfRange { value: i, levels })
            } else {
                Ok(())
            }
        }
        (OutputType::Score, Value::Int(_)) => Ok(()),
        (OutputType::Score, Value::Float(f)) if f.is_finite() => Ok(()),
        (OutputType::Score, Value::Float(_)) => Err(RecordError::NonFinite),
        _ => Err(mismatch()),
    }
}
