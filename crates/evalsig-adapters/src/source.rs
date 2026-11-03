//! Rows of the plain formats (JSON, JSON Lines, CSV), parsed within the limits.

use serde_json::Value as Json;

use crate::limits::{self, Limits};
use crate::mapping::Format;

/// Values with the source row they come from, numbered from 1 (lines for JSONL and
/// CSV, elements for JSON).
pub type Numbered = Vec<(usize, Json)>;

/// The rows of a file: a JSON document is one row, JSONL a row per line, CSV a row per
/// record as an object of strings keyed by the header. JSON is checked for depth first.
pub fn parse(format: Format, bytes: &[u8], limits: &Limits) -> Result<Numbered, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned())?;
    match format {
        Format::Json => limits::check_json_depth(bytes, limits),
        Format::Jsonl => limits::check_jsonl_depth(bytes, limits),
        Format::Csv => Ok(()),
    }
    .map_err(|e| e.to_string())?;
    match format {
        Format::Json => {
            let root = serde_json::from_str(text).map_err(|e| format!("line {}: {e}", e.line()))?;
            Ok(vec![(1, root)])
        }
        Format::Jsonl => text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| {
                serde_json::from_str(l)
                    .map(|v| (i + 1, v))
                    .map_err(|e| format!("line {}: {e}", i + 1))
            })
            .collect(),
        Format::Csv => Ok(crate::csv::parse(text)
            .map_err(|e| e.to_string())?
            .objects()),
    }
}

/// The elements of a JSON array, or the value itself, numbered from 1.
pub fn elements(root: Json) -> Numbered {
    let values = match root {
        Json::Array(xs) => xs,
        other => vec![other],
    };
    values
        .into_iter()
        .enumerate()
        .map(|(i, v)| (i + 1, v))
        .collect()
}

/// The scalar members of an object (strings, numbers, booleans), except `skip`: what a
/// harness's free-form metadata contributes as strata.
pub fn scalar_fields(v: &Json, skip: &[&str]) -> serde_json::Map<String, Json> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter(|(k, v)| {
            !skip.contains(&k.as_str())
                && matches!(v, Json::String(_) | Json::Number(_) | Json::Bool(_))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
