//! `map probe` (SPEC §5.3): the shape of records evalsig does not know yet, without
//! their contents. Per path: types, how many rows have it, how many distinct values
//! and, for strings, the longest length. Values are listed only for short categorical
//! paths with few values; free text and secret-like strings are never printed.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{Hash, Hasher};

use evalsig_core::validity::secret_shape;
use serde::Serialize;
use serde_json::Value as Json;

use crate::limits::{self, Limits};
use crate::mapping::Format;
use crate::source;

/// Paths with at most this many distinct short values have them listed.
pub const MAX_LISTED: usize = 20;
/// Longer strings are free text.
pub const MAX_LISTED_LEN: usize = 32;
/// Objects with more keys than this are maps keyed by data (IDs, names): their values
/// are walked under one `{*}` path, and the keys are not shown.
pub const MAX_KEYS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Json,
    Jsonl,
    Csv,
    Zip,
}

impl Shape {
    /// The shape a file's extension names, in any letter case.
    pub fn from_extension(path: &std::path::Path) -> Option<Shape> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "eval" | "zip" => Some(Shape::Zip),
            "json" => Some(Shape::Json),
            "jsonl" | "ndjson" => Some(Shape::Jsonl),
            "csv" => Some(Shape::Csv),
            _ => None,
        }
    }
}

/// A format evalsig has an adapter or recipe for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Known {
    pub format: &'static str,
    /// The `import --adapter` to use, if evalsig has one.
    pub adapter: Option<&'static str>,
    pub hint: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathStat {
    pub path: String,
    pub types: BTreeSet<&'static str>,
    /// Rows in which the path occurs.
    pub rows: usize,
    pub distinct: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    /// Why values are not listed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values_withheld: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub shape: Shape,
    pub rows: usize,
    pub known: Option<Known>,
    pub paths: Vec<PathStat>,
    /// Short scalar paths present in every row with a different value in each.
    pub key_candidates: Vec<String>,
    /// Small non-negative integers that repeat across rows.
    pub replicate_candidates: Vec<String>,
    /// Arrays of objects, for `source.records_path` of a JSON document.
    pub records_path_candidates: Vec<String>,
}

#[derive(Default)]
struct Acc {
    types: BTreeSet<&'static str>,
    rows: usize,
    /// The last row counted in `rows`, plus one (0 before any).
    last_row: usize,
    /// Hashes of every distinct value, to count them without keeping them.
    distinct: HashSet<u64>,
    /// The values themselves while few enough to be listed.
    values: BTreeSet<String>,
    max_len: Option<usize>,
    free_text: bool,
    secret: bool,
    /// The range of integer values, until a value that is not an integer.
    ints: Option<(i64, i64)>,
    non_int: bool,
}

fn kind(v: &Json) -> &'static str {
    match v {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

/// Visit a value at `path` (a buffer the walk extends and restores) in row `row`.
fn walk(
    v: &Json,
    path: &mut String,
    row: usize,
    stats: &mut BTreeMap<String, Acc>,
    arrays: &mut BTreeSet<String>,
) {
    if !stats.contains_key(path.as_str()) {
        stats.insert(path.clone(), Acc::default());
    }
    let acc = stats.get_mut(path.as_str()).expect("inserted above");
    acc.types.insert(kind(v));
    if acc.last_row != row + 1 {
        acc.last_row = row + 1;
        acc.rows += 1;
    }
    match v {
        Json::Object(m) => {
            let keyed_by_data = m.len() > MAX_KEYS;
            for (k, x) in m {
                let len = path.len();
                path.push('.');
                path.push_str(if keyed_by_data { "{*}" } else { k });
                walk(x, path, row, stats, arrays);
                path.truncate(len);
            }
        }
        Json::Array(xs) => {
            if xs.iter().any(Json::is_object) {
                arrays.insert(path.clone());
            }
            let len = path.len();
            path.push_str("[*]");
            for x in xs {
                walk(x, path, row, stats, arrays);
            }
            path.truncate(len);
        }
        scalar => {
            match scalar {
                Json::Number(n) if n.is_i64() => {
                    let i = n.as_i64().expect("an i64");
                    let (lo, hi) = acc.ints.unwrap_or((i, i));
                    acc.ints = Some((lo.min(i), hi.max(i)));
                }
                _ => acc.non_int = true,
            }
            let text = match scalar {
                Json::String(s) => {
                    let len = s.chars().count();
                    acc.max_len = Some(acc.max_len.unwrap_or(0).max(len));
                    acc.free_text |= len > MAX_LISTED_LEN;
                    // Once a path is withheld, its later values need no secret check.
                    if !acc.secret && !acc.free_text {
                        acc.secret = secret_shape(s).is_some();
                    }
                    Cow::Borrowed(s.as_str())
                }
                other => Cow::Owned(other.to_string()),
            };
            let mut h = DefaultHasher::new();
            text.hash(&mut h);
            if acc.distinct.insert(h.finish())
                && !acc.free_text
                && !acc.secret
                && acc.values.len() <= MAX_LISTED
            {
                acc.values.insert(text.into_owned());
            }
        }
    }
}

/// The rows of a file in the shape it has.
pub fn rows(shape: Shape, bytes: &[u8], limits: &Limits) -> Result<Vec<Json>, String> {
    let format = match shape {
        Shape::Zip => {
            let entries = limits::zip_entries(bytes, limits, |n| n.ends_with(".json"))
                .map_err(|e| e.to_string())?;
            return Ok(entries
                .iter()
                .filter_map(|(_, d)| serde_json::from_slice(d).ok())
                .collect());
        }
        Shape::Json => Format::Json,
        Shape::Jsonl => Format::Jsonl,
        Shape::Csv => Format::Csv,
    };
    let mut rows = source::parse(format, bytes, limits)?;
    if format == Format::Csv {
        // Cells are text; read them as a plain CSV import would, so numbers count as
        // numbers for key, replicate and value candidates.
        for (_, row) in &mut rows {
            if let Json::Object(m) = row {
                for v in m.values_mut() {
                    if let Some(typed) = v
                        .as_str()
                        .and_then(|s| crate::table::map_value(s, None, None).ok())
                    {
                        *v = typed;
                    }
                }
            }
        }
    }
    Ok(match format {
        Format::Json => source::elements(
            rows.into_iter()
                .next()
                .map(|(_, v)| v)
                .unwrap_or(Json::Null),
        ),
        _ => rows,
    }
    .into_iter()
    .map(|(_, v)| v)
    .collect())
}

/// Recognise a format by the keys its records always have.
pub fn fingerprint(shape: Shape, rows: &[&Json]) -> Option<Known> {
    let first = *rows.first()?;
    let has = |keys: &[&str]| keys.iter().all(|k| first.get(k).is_some());
    let inspect = Known {
        format: "inspect",
        adapter: Some("inspect"),
        hint: "an Inspect log: evalsig import --adapter inspect <file>",
    };
    if shape == Shape::Zip
        && rows
            .iter()
            .any(|r| r.get("eval").is_some() && r.get("version").is_some())
    {
        return Some(inspect);
    }
    if has(&["version", "eval"])
        && (first.get("samples").is_some() || first.get("results").is_some())
    {
        return Some(inspect);
    }
    if has(&["evalId", "results"]) && first["results"].get("version").is_some() {
        return Some(Known {
            format: "promptfoo",
            adapter: Some("promptfoo"),
            hint: "promptfoo results: evalsig import --adapter promptfoo <file>",
        });
    }
    if has(&["doc_id", "doc", "resps", "filtered_resps"]) {
        return Some(Known {
            format: "lm-evaluation-harness",
            adapter: None,
            hint: "lm-evaluation-harness samples (--log_samples): map doc_id to the item and the metric field to the value",
        });
    }
    if has(&["traceId", "name", "dataType"]) {
        return Some(Known {
            format: "langfuse-scores",
            adapter: None,
            hint: "a Langfuse score export: see `evalsig recipes acquire langfuse`",
        });
    }
    None
}

/// Probe rows. With `no_values`, no value is listed at all.
pub fn probe<'r>(shape: Shape, rows: impl IntoIterator<Item = &'r Json>, no_values: bool) -> Probe {
    let rows: Vec<&Json> = rows.into_iter().collect();
    let mut stats: BTreeMap<String, Acc> = BTreeMap::new();
    let mut arrays = BTreeSet::new();
    let mut path = String::from("$");
    for (i, row) in rows.iter().enumerate() {
        walk(row, &mut path, i, &mut stats, &mut arrays);
    }
    let n = rows.len();
    let mut key_candidates = Vec::new();
    let mut replicate_candidates = Vec::new();
    let paths = stats
        .into_iter()
        .filter(|(p, _)| p != "$")
        .map(|(path, a)| {
            let scalar = a.types.iter().all(|t| matches!(*t, "string" | "integer"));
            let distinct = a.distinct.len();
            if n > 1
                && scalar
                && !a.free_text
                && a.rows == n
                && distinct == n
                && !path.contains("[*]")
            {
                key_candidates.push(path.clone());
            }
            if let (Some((lo, hi)), false) = (a.ints, a.non_int)
                && lo >= 0
                && hi <= 20
                && distinct >= 2
                && distinct < a.rows
            {
                replicate_candidates.push(path.clone());
            }
            let withheld = if no_values {
                Some("--no-values")
            } else if a.secret {
                Some("secret-like")
            } else if a.free_text {
                Some("free text")
            } else if distinct > MAX_LISTED {
                Some("too many values")
            } else {
                None
            };
            let listed = distinct > 0 && withheld.is_none();
            PathStat {
                path,
                types: a.types,
                rows: a.rows,
                distinct,
                max_len: a.max_len,
                values: listed.then(|| a.values.into_iter().collect()),
                values_withheld: withheld.filter(|_| distinct > 0),
            }
        })
        .collect();
    Probe {
        shape,
        rows: n,
        known: fingerprint(shape, &rows),
        paths,
        key_candidates,
        replicate_candidates,
        records_path_candidates: arrays.into_iter().map(|p| format!("{p}[*]")).collect(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/../../tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn known(shape: Shape, bytes: &[u8]) -> Option<&'static str> {
        let rows = rows(shape, bytes, &Limits::default()).unwrap();
        fingerprint(shape, &rows.iter().collect::<Vec<_>>()).map(|k| k.format)
    }

    #[test]
    fn known_formats_are_fingerprinted() {
        assert_eq!(
            known(Shape::Zip, &fixture("harness/inspect/probe.eval")),
            Some("inspect")
        );
        assert_eq!(
            known(Shape::Json, &fixture("harness/inspect/probe.json")),
            Some("inspect")
        );
        assert_eq!(
            known(Shape::Json, &fixture("harness/promptfoo/out.json")),
            Some("promptfoo")
        );
        let lm = br#"{"doc_id": 0, "doc": {"q": "x"}, "target": "y", "arguments": [], "resps": [], "filtered_resps": [], "acc": 1.0}"#;
        assert_eq!(known(Shape::Jsonl, lm), Some("lm-evaluation-harness"));
        let lf = br#"[{"id": "s1", "traceId": "t1", "name": "helpfulness", "value": 1, "dataType": "NUMERIC", "source": "ANNOTATION"}]"#;
        assert_eq!(known(Shape::Json, lf), Some("langfuse-scores"));
        assert_eq!(known(Shape::Jsonl, br#"{"a": 1}"#), None);
    }

    #[test]
    fn free_text_and_secrets_are_shown_as_types_and_lengths_only() {
        let rows: Vec<Json> = (0..6)
            .map(|i| {
                json!({
                    "id": format!("t-{i}"),
                    "trial": i % 2,
                    "label": if i % 3 == 0 { "yes" } else { "no" },
                    "note": format!("The customer Jane Roe wrote a long complaint number {i}"),
                    "token": "sk-aaaaaaaaaaaaaaaaaaaaaaaa",
                })
            })
            .collect();
        let p = probe(Shape::Jsonl, &rows, false);
        let text = serde_json::to_string(&p).unwrap();
        assert!(!text.contains("Jane") && !text.contains("sk-aaa"), "{text}");
        let by = |path: &str| p.paths.iter().find(|s| s.path == path).unwrap();
        assert_eq!(by("$.note").values_withheld, Some("free text"));
        assert_eq!(by("$.note").max_len, Some(53));
        assert_eq!(by("$.token").values_withheld, Some("secret-like"));
        assert_eq!(by("$.label").values, Some(vec!["no".into(), "yes".into()]));
        assert_eq!(p.key_candidates, ["$.id"]);
        assert_eq!(p.replicate_candidates, ["$.trial"]);
        let quiet = serde_json::to_string(&probe(Shape::Jsonl, &rows, true)).unwrap();
        assert!(!quiet.contains("\"yes\""));
    }
}

#[cfg(test)]
mod more_tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn many_rows_keep_their_key_and_data_keys_stay_hidden() {
        let rows: Vec<Json> = (0..12_000)
            .map(
                |i| json!({"id": format!("t-{i}"), "by_user": {format!("user{i}@example.com"): i}}),
            )
            .collect();
        let p = probe(Shape::Jsonl, &rows, false);
        assert_eq!(p.key_candidates, ["$.id"]);
        let wide: Json = Json::Object(
            (0..200)
                .map(|i| (format!("user{i}@example.com"), json!(i)))
                .collect(),
        );
        let q = probe(Shape::Json, [&wide], false);
        let text = serde_json::to_string(&q).unwrap();
        assert!(!text.contains("example.com"), "{text}");
        assert!(
            q.paths
                .iter()
                .any(|s| s.path == "${*}" || s.path == "$.{*}")
        );
    }

    #[test]
    fn csv_numbers_count_as_numbers() {
        let rows = rows(
            Shape::Csv,
            b"id,trial,score\na,0,0.1\na,1,0.2\nb,0,0.3\nb,1,0.4\n",
            &Limits::default(),
        )
        .unwrap();
        let p = probe(Shape::Csv, &rows, false);
        assert_eq!(p.replicate_candidates, ["$.trial"]);
        assert!(Shape::from_extension(std::path::Path::new("A.JSONL")).is_some());
    }
}
