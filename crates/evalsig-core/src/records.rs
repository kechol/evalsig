//! The canonical record format `records/1` (D-003).
//!
//! Optional fields stay `None` when absent so that a record serializes back to what
//! was read; defaults from the schema are applied by accessor methods instead.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use crate::time::{Instant, TimestampError};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strata: Option<Map<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_inclusion_prob: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_ref: Option<HashRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<LabelKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_replicate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labeled_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Judgment {
    pub item_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub gen_replicate: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_replicate: Option<u32>,
    pub output_type: OutputType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<Feature>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<HashRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale_ref: Option<HashRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judged_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Map<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub left_run_id: String,
    pub right_run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_gen_replicate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_gen_replicate: Option<u32>,
    pub rater_id: String,
    pub source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_replicate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<PreferenceOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale_ref: Option<HashRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judged_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<GraderKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<HashRef>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HarnessRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub run_id: String,
    pub system: SystemRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grader: Option<GraderRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_semantics: Option<ValueSemantics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_type: Option<OutputType>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored: Option<Stored>,
}

/// A judgment or label value. A number with no fractional part is an integer, so
/// `2` and `2.0`, which have the same canonical form, are the same value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = Value;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a boolean or a number")
            }
            fn visit_bool<E>(self, b: bool) -> Result<Value, E> {
                Ok(Value::Bool(b))
            }
            fn visit_i64<E>(self, i: i64) -> Result<Value, E> {
                Ok(Value::Int(i))
            }
            fn visit_u64<E: serde::de::Error>(self, u: u64) -> Result<Value, E> {
                i64::try_from(u)
                    .map(Value::Int)
                    .map_err(|_| E::custom("integer out of range"))
            }
            fn visit_f64<E>(self, f: f64) -> Result<Value, E> {
                // Only integral values a double holds exactly as an integer (|f| ≤ 2^53 − 1)
                // become integers; canonical integers are limited to that range.
                if f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0 {
                    Ok(Value::Int(f as i64))
                } else {
                    Ok(Value::Float(f))
                }
            }
        }
        d.deserialize_any(V)
    }
}

impl Value {
    /// A score from a harness. NaN and infinities have no JSON representation and
    /// become missing (`invalid_output`) instead of a number.
    pub fn from_score(x: f64) -> Result<Value, MissingReason> {
        if x.is_finite() {
            Ok(Value::Float(x))
        } else {
            Err(MissingReason::InvalidOutput)
        }
    }

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[error(transparent)]
    Timestamp(#[from] TimestampError),
    #[error("{0}")]
    OutOfRange(&'static str),
}

impl RecordError {
    /// The stable issue code reported by `evalsig validate` (D-012).
    pub fn code(&self) -> &'static str {
        match self {
            RecordError::Json(_) => "record.invalid",
            RecordError::ValueAndMissing => "record.value_and_missing",
            RecordError::NeitherValueNorMissing => "record.neither_value_nor_missing",
            RecordError::ValueType { .. } => "record.value_type",
            RecordError::MissingLevels => "record.missing_levels",
            RecordError::LevelOutOfRange { .. } => "record.level_out_of_range",
            RecordError::TooFewLevels => "record.too_few_levels",
            RecordError::NonFinite => "record.non_finite",
            RecordError::Timestamp(_) => "record.timestamp",
            RecordError::OutOfRange(_) => "record.out_of_range",
        }
    }
}

impl Record {
    /// The missing marker of a judgment, label or preference.
    pub fn missing(&self) -> Option<&Missing> {
        match self {
            Record::Judgment(j) => j.missing.as_ref(),
            Record::Label(l) => l.missing.as_ref(),
            Record::Preference(p) => p.missing.as_ref(),
            Record::Item(_) | Record::Run(_) | Record::Content(_) => None,
        }
    }

    /// Parse one JSONL line and check the rules the type system does not express.
    pub fn from_json(line: &str) -> Result<Record, RecordError> {
        let record: Record = serde_json::from_str(line)?;
        record.check()?;
        Ok(record)
    }

    /// Like [`Record::from_json`], for a value that is already parsed.
    pub fn from_value(value: serde_json::Value) -> Result<Record, RecordError> {
        let record: Record = serde_json::from_value(value)?;
        record.check()?;
        Ok(record)
    }

    /// Check value-xor-missing, that values match the declared output type, and that
    /// timestamps are RFC 3339.
    pub fn check(&self) -> Result<(), RecordError> {
        match self {
            Record::Judgment(j) => {
                if let Observation::Value(v) = j.observation()? {
                    check_value(j.output_type, j.levels, *v)?;
                }
                check_time(&j.judged_at)?;
                check_levels(j.levels)
            }
            Record::Label(l) => {
                if let (Observation::Value(v), Some(levels)) = (l.observation()?, l.levels) {
                    check_value(OutputType::Ordinal, Some(levels), *v)?;
                }
                check_time(&l.labeled_at)?;
                check_levels(l.levels)
            }
            Record::Preference(p) => {
                p.observation()?;
                check_time(&p.judged_at)
            }
            Record::Run(r) => {
                check_time(&r.started_at)?;
                check_levels(r.levels)
            }
            Record::Item(i) => {
                if i.weight.is_some_and(|w| !(w > 0.0 && w <= 100.0)) {
                    return Err(RecordError::OutOfRange("weight must lie in (0, 100]"));
                }
                if i.label_inclusion_prob
                    .is_some_and(|p| !(p > 0.0 && p <= 1.0))
                {
                    return Err(RecordError::OutOfRange(
                        "label_inclusion_prob must lie in (0, 1]",
                    ));
                }
                check_time(&i.occurred_at)
            }
            Record::Content(_) => Ok(()),
        }
    }
}

fn check_time(ts: &Option<Timestamp>) -> Result<(), RecordError> {
    if let Some(t) = ts {
        Instant::parse(t)?;
    }
    Ok(())
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
