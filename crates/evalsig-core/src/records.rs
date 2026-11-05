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
    /// An item: one object being judged.
    Item(Item),
    /// A reference label for an item from one rater.
    Label(Label),
    /// A judgment: one output of the system under test for an item.
    Judgment(Judgment),
    /// A preference: a pairwise comparison of two runs' outputs on an item.
    Preference(Preference),
    /// A run: one import unit, the judgments of one system version on one dataset.
    Run(Run),
    /// A content blob referenced by hash from other records.
    Content(Content),
}

/// One object being judged, such as an Inspect sample or a promptfoo test case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Identifier of the item, unique within a dataset (1 to 512 characters).
    pub item_id: String,
    /// Group of correlated items; the resampling and splitting unit. Absent means
    /// the item itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,
    /// Stratum name to stratum value, for reporting groups of items separately.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strata: Option<Map<String, Json>>,
    /// Weight that maps the item back to the population; greater than 0 and at most
    /// 100. Absent means 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// Probability in (0, 1] that the item received a human reference label; used
    /// to correct verification bias by weighting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_inclusion_prob: Option<f64>,
    /// When the item occurred; used for time-based splits and trends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<Timestamp>,
    /// Reference to the system-independent input only, without prompt template or
    /// system prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_ref: Option<HashRef>,
    /// User attributes that are kept but not used in analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Json>>,
    /// Reference to reference text, kept as content and not used by metrics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_ref: Option<HashRef>,
}

impl Item {
    /// The item weight, 1 when absent.
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
    /// Produced by a human rater.
    Human,
    /// Produced by a model rater.
    Model,
    /// Produced by a panel's consensus.
    Consensus,
    /// Produced outside the rater categories above.
    External,
}

/// How a reference label relates to other labels of the same item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelKind {
    /// Made without seeing other labels.
    #[default]
    Independent,
    /// Corrects the rater's own earlier label.
    Revision,
    /// Resolves a split after seeing the other labels, so it is not independent.
    Adjudication,
}

/// A reference label: one rater's judgment treated as correct for an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    /// Identifier of the labelled item.
    pub item_id: String,
    /// Identifier of the rater that produced the label.
    pub rater_id: String,
    /// Whether a human, a model or a panel produced the label.
    pub source: Source,
    /// Task name when the item has several judgment tasks. Absent means `default`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Label kind. Absent means independent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<LabelKind>,
    /// Number of a blind re-label of the same item by the same rater; 0 is the
    /// first. Absent means 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_replicate: Option<u32>,
    /// The label value. Exactly one of `value` and `missing` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Why the label could not be obtained. Exactly one of `value` and `missing` is
    /// present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    /// Revision number (at least 1) for the same item, rater, task and label
    /// replicate; revisions are appended.
    pub version: u32,
    /// Free-text reason for the label (at most 4000 characters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When the label was made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labeled_at: Option<Timestamp>,
    /// Reference to the original record in the source system; not used in analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    /// Number of levels (at least 2) of an ordinal label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
}

impl Label {
    /// The task name, `default` when absent.
    pub fn task(&self) -> &str {
        self.task.as_deref().unwrap_or(DEFAULT_TASK)
    }

    /// The label kind, independent when absent.
    pub fn kind(&self) -> LabelKind {
        self.kind.unwrap_or_default()
    }

    /// The label replicate number, 0 when absent.
    pub fn label_replicate(&self) -> u32 {
        self.label_replicate.unwrap_or(0)
    }

    /// The label's value or missing marker; an error unless exactly one is present.
    pub fn observation(&self) -> Result<Observation<'_, Value>, RecordError> {
        Observation::from_parts(self.value.as_ref(), self.missing.as_ref())
    }
}

/// A feature (rule, aspect, assertion) that fired inside a judgment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    /// Identifier of the feature (1 to 512 characters).
    pub feature_id: String,
    /// Whether the feature fired in this judgment.
    pub fired: bool,
    /// How strongly the feature fired, on the feature's own scale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
}

/// One output of the system under test for one item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Judgment {
    /// Identifier of the judged item.
    pub item_id: String,
    /// Identifier of the run that produced the judgment.
    pub run_id: String,
    /// Task name when the item has several judgment tasks. Absent means `default`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Generation replicate: which re-run of the system on the same item, counted from 0.
    pub gen_replicate: u32,
    /// Grading replicate: which re-grading of the same output, counted from 0.
    /// Absent means 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_replicate: Option<u32>,
    /// Output type of the value; constant within a run.
    pub output_type: OutputType,
    /// Number of ordinal levels (at least 2); required for an ordinal value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
    /// The judgment value. Exactly one of `value` and `missing` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Why the judgment has no value. Exactly one of `value` and `missing` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    /// Features that fired or did not fire inside this judgment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<Feature>>,
    /// Reference to the output text being judged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<HashRef>,
    /// Reference to text such as the grader's reasoning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale_ref: Option<HashRef>,
    /// When the judgment was made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judged_at: Option<Timestamp>,
    /// Tokens, money and time spent; not used in analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Map<String, Json>>,
    /// Reference to the original record in the source system; not used in analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

impl Judgment {
    /// The task name, `default` when absent.
    pub fn task(&self) -> &str {
        self.task.as_deref().unwrap_or(DEFAULT_TASK)
    }

    /// The grading replicate number, 0 when absent.
    pub fn grade_replicate(&self) -> u32 {
        self.grade_replicate.unwrap_or(0)
    }

    /// The judgment's value or missing marker; an error unless exactly one is present.
    pub fn observation(&self) -> Result<Observation<'_, Value>, RecordError> {
        Observation::from_parts(self.value.as_ref(), self.missing.as_ref())
    }
}

/// Outcome of a pairwise comparison, by presentation side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceOutcome {
    /// The output presented first (left) is preferred.
    Left,
    /// The output presented second (right) is preferred.
    Right,
    /// Neither output is preferred.
    Tie,
}

/// A comparison of the outputs of two runs on the same item, saying which is better.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preference {
    /// Identifier of the compared item.
    pub item_id: String,
    /// Task name when the item has several judgment tasks. Absent means `default`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Run of the output presented first (left).
    pub left_run_id: String,
    /// Run of the output presented second (right).
    pub right_run_id: String,
    /// Generation replicate of the left output. Absent means 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_gen_replicate: Option<u32>,
    /// Generation replicate of the right output. Absent means 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_gen_replicate: Option<u32>,
    /// Identifier of the rater that compared the outputs.
    pub rater_id: String,
    /// Whether a human, a model or a panel produced the preference.
    pub source: Source,
    /// Grading replicate of this comparison. Absent means 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_replicate: Option<u32>,
    /// Which side is preferred. Exactly one of `outcome` and `missing` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<PreferenceOutcome>,
    /// Why the comparison has no outcome. Exactly one of `outcome` and `missing` is
    /// present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    /// Reference to text such as the rater's reasoning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale_ref: Option<HashRef>,
    /// When the comparison was made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judged_at: Option<Timestamp>,
    /// Reference to the original record in the source system; not used in analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

impl Preference {
    /// The task name, `default` when absent.
    pub fn task(&self) -> &str {
        self.task.as_deref().unwrap_or(DEFAULT_TASK)
    }

    /// The preference's outcome or missing marker; an error unless exactly one is present.
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

impl ValueSemantics {
    /// The lowercase name used in JSON and output: `decision` or `score`.
    pub fn as_str(self) -> &'static str {
        match self {
            ValueSemantics::Decision => "decision",
            ValueSemantics::Score => "score",
        }
    }
}

/// Type of a run's values; constant within a run and checked against each judgment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputType {
    /// A boolean value.
    Binary,
    /// An integer value from 0 (larger is more positive) below `levels`.
    Ordinal,
    /// A finite number.
    Score,
}

/// The system under test that produced a run's judgments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemRef {
    /// Identifier of the system.
    pub id: String,
    /// Version of the system.
    pub version: String,
    /// Hash of the system's configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<HashRef>,
}

/// What kind of grader produced the judgments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraderKind {
    /// A model grader.
    Model,
    /// A rule-based grader.
    Rule,
    /// A human grader.
    Human,
    /// A combination of grader kinds.
    Mixed,
}

/// The grader that graded a run's outputs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GraderRef {
    /// Identifier of the grader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Version of the grader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Kind of the grader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<GraderKind>,
    /// Hash of the grader prompt or code; compared with the approval at the gate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<HashRef>,
}

/// The harness that produced a run.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HarnessRef {
    /// Name of the harness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Version of the harness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Reference to the run in the harness's own records.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<String>,
}

/// One import unit: the judgments of one system version on one dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// Identifier of the run.
    pub run_id: String,
    /// The system under test.
    pub system: SystemRef,
    /// The grader; required when value semantics is `score`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grader: Option<GraderRef>,
    /// The harness that produced the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessRef>,
    /// When the run started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<Timestamp>,
    /// User attributes that are kept but not used in analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attrs: Option<Map<String, Json>>,
    /// Value semantics of the run. Absent means decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_semantics: Option<ValueSemantics>,
    /// Output type of the run's judgments; constant within the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_type: Option<OutputType>,
    /// Number of ordinal levels (at least 2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
}

impl Run {
    /// The value semantics, decision when absent.
    pub fn value_semantics(&self) -> ValueSemantics {
        self.value_semantics.unwrap_or_default()
    }
}

/// How much of a content blob is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stored {
    /// The full content is kept.
    Full,
    /// Only the hash is kept.
    Hash,
    /// The content is kept with sensitive parts redacted.
    Redact,
    /// The content was removed after being stored.
    Purged,
}

/// A content blob, such as an input, an output or a grader's reasoning, identified by
/// hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    /// Content-hash reference that other records use to point at this blob.
    #[serde(rename = "ref")]
    pub content_ref: HashRef,
    /// Media type of the blob.
    pub media_type: String,
    /// Size of the blob in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// How much of the blob is kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored: Option<Stored>,
}

/// A judgment or label value. A number with no fractional part is an integer, so
/// `2` and `2.0`, which have the same canonical form, are the same value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    /// A boolean; the value of a binary judgment or label.
    Bool(bool),
    /// An integer; the value of an ordinal judgment or label, counted from 0.
    Int(i64),
    /// A finite number that is not integral or exceeds the exact integer range.
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
    /// The system failed with an error.
    Error,
    /// The system or grader timed out.
    Timeout,
    /// The system refused to answer.
    Refused,
    /// The output could not be read as a valid value.
    InvalidOutput,
    /// A resource limit such as a token or cost limit was reached.
    Limit,
    /// The grader failed to grade the output.
    GraderFailed,
    /// The item was not run.
    NotRun,
    /// The judgment was invalidated after it was made.
    Invalidated,
    /// The item was excluded from the run.
    Excluded,
    /// Any other reason; see the detail.
    Other,
}

impl MissingReason {
    /// Every missing reason, in declaration order.
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

/// A marker that a judgment, label or preference has no value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Missing {
    /// Why the value is missing.
    pub reason: MissingReason,
    /// Free-text detail (at most 4000 characters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Either a value or a missing marker; a record carries exactly one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Observation<'a, V> {
    /// A present value.
    Value(&'a V),
    /// A missing marker.
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

/// A record that is malformed or violates a `records/1` constraint.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// The line is not valid JSON for the record.
    #[error("invalid record: {0}")]
    Json(#[from] serde_json::Error),
    /// The record has both `value` and `missing`.
    #[error("record has both `value` and `missing`")]
    ValueAndMissing,
    /// The record has neither `value` nor `missing`.
    #[error("record has neither `value` nor `missing`")]
    NeitherValueNorMissing,
    /// The value type does not match the output type.
    #[error("value {value:?} does not match output type {output_type:?}")]
    ValueType {
        /// The output type the value was checked against.
        output_type: OutputType,
        /// The offending value.
        value: Value,
    },
    /// An ordinal record has no `levels`.
    #[error("ordinal record requires `levels`")]
    MissingLevels,
    /// An ordinal value lies outside 0 to `levels` - 1.
    #[error("ordinal value {value} is outside 0..{levels}")]
    LevelOutOfRange {
        /// The offending value.
        value: i64,
        /// The number of levels.
        levels: u32,
    },
    /// `levels` is less than 2.
    #[error("`levels` must be at least 2")]
    TooFewLevels,
    /// A score value is NaN or infinite.
    #[error("score value must be finite")]
    NonFinite,
    /// A timestamp is not valid RFC 3339.
    #[error(transparent)]
    Timestamp(#[from] TimestampError),
    /// A numeric field is outside its allowed range; the message names it.
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
