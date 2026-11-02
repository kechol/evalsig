//! Line-by-line validation of `records/1` JSONL with stable error codes (D-012).
//!
//! Each line is checked in three steps: JSON that canonicalizes (no duplicate keys,
//! integers in the exact range), the JSON Schema of its record kind, and the rules the
//! schema cannot express (ordinal levels). The first failing step reports its issues.

use std::collections::BTreeMap;

use evalsig_core::records::{Record, RecordError};
use evalsig_store::canonical::{Canonical, CanonicalError};
use serde::Serialize;
use serde_json::Value;

use crate::schemas;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub line: usize,
    pub code: String,
    /// JSON Pointer into the record; empty for the whole line.
    pub path: String,
    pub message: String,
}

pub struct RecordValidator {
    per_kind: BTreeMap<String, jsonschema::Validator>,
}

impl RecordValidator {
    pub fn new() -> Self {
        let schema: Value =
            serde_json::from_str(schemas::RECORDS).expect("embedded schema is JSON");
        let kinds: Vec<String> = schema["$defs"]
            .as_object()
            .expect("schema has $defs")
            .iter()
            .filter(|(_, d)| d["properties"]["record"]["const"].is_string())
            .map(|(k, _)| k.clone())
            .collect();
        let per_kind = kinds
            .into_iter()
            .map(|kind| {
                // Validate against the kind's definition directly, so errors name the
                // failing keyword instead of the top-level `oneOf`.
                let mut s = schema.clone();
                let obj = s.as_object_mut().expect("schema is an object");
                obj.remove("oneOf");
                obj.insert("$ref".into(), Value::String(format!("#/$defs/{kind}")));
                let v = jsonschema::options()
                    .should_validate_formats(true)
                    .build(&s)
                    .expect("embedded schema compiles");
                (kind, v)
            })
            .collect();
        RecordValidator { per_kind }
    }

    pub fn validate_line(&self, line_no: usize, line: &str) -> Vec<Issue> {
        let issue = |code: &str, path: &str, message: String| Issue {
            line: line_no,
            code: code.to_owned(),
            path: path.to_owned(),
            message,
        };
        if let Err(e) = Canonical::parse(line) {
            let code = match e {
                CanonicalError::DuplicateKey(_) | CanonicalError::DuplicateKeyAfterNfc(_) => {
                    "json.duplicate_key"
                }
                CanonicalError::IntegerOutOfRange(_) => "json.integer_range",
                CanonicalError::Json(_) | CanonicalError::NonFinite => "json.invalid",
            };
            return vec![issue(code, "", e.to_string())];
        }
        let value: Value = serde_json::from_str(line).expect("canonical JSON parses");
        let Some(obj) = value.as_object() else {
            return vec![issue(
                "record.not_object",
                "",
                "a record must be a JSON object".into(),
            )];
        };
        let Some(kind) = obj.get("record").and_then(Value::as_str) else {
            return vec![issue(
                "record.missing_kind",
                "/record",
                "a record needs a string `record` field".into(),
            )];
        };
        let Some(validator) = self.per_kind.get(kind) else {
            let known: Vec<&str> = self.per_kind.keys().map(String::as_str).collect();
            return vec![issue(
                "record.unknown_kind",
                "/record",
                format!(
                    "unknown record kind {kind:?}; expected one of {}",
                    known.join(", ")
                ),
            )];
        };
        let schema_issues: Vec<Issue> = validator
            .iter_errors(&value)
            .map(|e| {
                issue(
                    &format!("schema.{}", snake(e.kind().keyword())),
                    &e.instance_path().to_string(),
                    e.to_string(),
                )
            })
            .collect();
        if !schema_issues.is_empty() {
            return schema_issues;
        }
        match Record::from_json(line) {
            Ok(_) => Vec::new(),
            Err(e) => vec![issue(record_code(&e), "", e.to_string())],
        }
    }
}

fn record_code(e: &RecordError) -> &'static str {
    match e {
        RecordError::Json(_) => "record.invalid",
        RecordError::ValueAndMissing => "record.value_and_missing",
        RecordError::NeitherValueNorMissing => "record.neither_value_nor_missing",
        RecordError::ValueType { .. } => "record.value_type",
        RecordError::MissingLevels => "record.missing_levels",
        RecordError::LevelOutOfRange { .. } => "record.level_out_of_range",
        RecordError::TooFewLevels => "record.too_few_levels",
        RecordError::NonFinite => "record.non_finite",
    }
}

fn snake(keyword: &str) -> String {
    let mut out = String::new();
    for c in keyword.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(line: &str) -> Vec<String> {
        RecordValidator::new()
            .validate_line(1, line)
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    #[test]
    fn every_example_is_valid() {
        let v = RecordValidator::new();
        for (i, line) in include_str!("../../../examples/records.jsonl")
            .lines()
            .enumerate()
        {
            assert_eq!(v.validate_line(i + 1, line), [], "line {}", i + 1);
        }
    }

    #[test]
    fn malformed_records_fail_with_stable_codes() {
        let cases = [
            ("not json", "json.invalid"),
            (
                r#"{"record":"item","item_id":"a","item_id":"b"}"#,
                "json.duplicate_key",
            ),
            (
                r#"{"record":"item","item_id":"a","weight":9007199254740993}"#,
                "json.integer_range",
            ),
            ("[1, 2]", "record.not_object"),
            (r#"{"item_id":"a"}"#, "record.missing_kind"),
            (r#"{"record":"verdict"}"#, "record.unknown_kind"),
            (r#"{"record":"item"}"#, "schema.required"),
            (
                r#"{"record":"item","item_id":"a","colour":"red"}"#,
                "schema.additional_properties",
            ),
            (r#"{"record":"item","item_id":7}"#, "schema.type"),
            (
                r#"{"record":"item","item_id":"a","weight":0}"#,
                "schema.exclusive_minimum",
            ),
            (
                r#"{"record":"item","item_id":"a","input_ref":"md5:00"}"#,
                "schema.pattern",
            ),
            (
                r#"{"record":"item","item_id":"a","occurred_at":"yesterday"}"#,
                "schema.format",
            ),
            (
                r#"{"record":"label","item_id":"a","rater_id":"r","source":"oracle","version":1,"value":true}"#,
                "schema.enum",
            ),
            (
                r#"{"record":"label","item_id":"a","rater_id":"r","source":"human","version":1}"#,
                "schema.one_of",
            ),
            (
                r#"{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"ordinal","levels":4,"value":4}"#,
                "record.level_out_of_range",
            ),
        ];
        for (line, code) in cases {
            let got = codes(line);
            assert!(
                got.iter().any(|c| c == code),
                "{line}: expected {code}, got {got:?}"
            );
        }
    }
}
