//! `map test` (SPEC §5.3): how much a mapping loses. Coverage of the source rows, paths
//! the mapping never reads, key integrity, the distribution of mapped values, and the
//! expectations of `mapping.tests.jsonl`.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::mapping::{self, Compiled, Host, Issue, MapError};
use crate::probe::{self, Shape};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub rows: usize,
    /// Rows that gave records without a problem.
    pub mapped: usize,
    /// Rows left out, by the field that failed.
    pub dropped: usize,
    pub dropped_by: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Integrity {
    /// Judgments beyond the first with the same (item, run, task, replicates).
    pub duplicate_judgments: usize,
    /// Item IDs mapped to different item records.
    pub conflicting_items: usize,
    /// (item, run, task) groups whose generation replicates are not 0, 1, …, n-1.
    pub replicate_gaps: usize,
    /// Items without a label, when the mapping maps labels.
    pub items_without_label: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Values {
    /// Judgment values and how often each occurs.
    pub judgments: BTreeMap<String, usize>,
    /// Missing judgments by reason.
    pub missing: BTreeMap<String, usize>,
    /// Source values `value_map` did not cover, with counts.
    pub not_mapped: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseFailure {
    pub line: usize,
    pub expected: Vec<Json>,
    pub got: Vec<Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Cases {
    pub cases: usize,
    pub passed: usize,
    pub failures: Vec<CaseFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub coverage: Coverage,
    pub unused_paths: Vec<String>,
    pub integrity: Integrity,
    pub values: Values,
}

/// `$` and `@` paths written anywhere in the mapping file, as segment patterns.
fn written_paths(mapping_text: &str) -> (Vec<Vec<String>>, BTreeSet<String>) {
    let chars: Vec<char> = mapping_text.chars().collect();
    let (mut rooted, mut relative) = (Vec::new(), BTreeSet::new());
    let mut i = 0;
    while i < chars.len() {
        if (chars[i] == '$' || chars[i] == '@') && chars.get(i + 1) == Some(&'.') {
            let root = chars[i];
            let mut depth = 0i32;
            let mut j = i + 1;
            while j < chars.len() {
                match chars[j] {
                    '{' | '[' => depth += 1,
                    '}' | ']' if depth == 0 => break,
                    '}' | ']' => depth -= 1,
                    '"' | '\'' | ' ' | ',' | '\n' | '(' | ')' if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            let token: String = chars[i + 2..j].iter().collect();
            let segments = segments(&token);
            // A path that starts with an insertion reads a joined file, not the row.
            if root == '$' && segments.first().is_some_and(|s| s != "*") {
                rooted.push(segments);
            } else if let Some(last) = segments.last() {
                relative.insert(last.clone());
            }
        }
        // Step one character, so paths inside this one (filters, insertions) are read too.
        i += 1;
    }
    (rooted, relative)
}

/// Path segments without brackets; `{...}` insertions become `*`.
fn segments(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    for c in path.chars() {
        match c {
            '[' | '{' => {
                if c == '{' && depth == 0 {
                    cur.push('*');
                }
                depth += 1;
            }
            ']' | '}' => depth -= 1,
            '.' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ if depth == 0 => cur.push(c),
            _ => {}
        }
    }
    out.push(cur);
    out.retain(|s| !s.is_empty());
    out
}

fn matches(pattern: &[String], path: &[String]) -> bool {
    pattern.len() <= path.len()
        && pattern
            .iter()
            .zip(path)
            .all(|(p, s)| p == s || p.contains('*'))
}

/// Leaf paths of the source that the mapping never reads.
fn unused(rows: &[(usize, Json)], mapping_text: &str) -> Vec<String> {
    let values: Vec<Json> = rows.iter().map(|(_, r)| r.clone()).collect();
    let probed = probe::probe(Shape::Jsonl, &values, true);
    let (rooted, relative) = written_paths(mapping_text);
    probed
        .paths
        .into_iter()
        .filter(|s| !s.types.iter().any(|t| *t == "object" || *t == "array"))
        .map(|s| s.path)
        .filter(|p| {
            let segs = segments(p.trim_start_matches('$').trim_start_matches('.'));
            let read = rooted.iter().any(|pat| matches(pat, &segs))
                || segs.last().is_some_and(|l| relative.contains(l));
            !read
        })
        .collect()
}

fn text(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The report of mapping these rows.
pub fn report(
    rows: &[(usize, Json)],
    records: &[(usize, Json)],
    issues: &[Issue],
    maps_labels: bool,
    mapping_text: &str,
) -> Report {
    let failed: BTreeSet<usize> = issues.iter().map(|i| i.row).collect();
    let mut dropped_by = BTreeMap::new();
    for i in issues {
        *dropped_by.entry(i.at.clone()).or_insert(0) += 1;
    }
    let coverage = Coverage {
        rows: rows.len(),
        mapped: rows.iter().filter(|(n, _)| !failed.contains(n)).count(),
        dropped: failed.len(),
        dropped_by,
    };
    let mut integrity = Integrity::default();
    let mut values = Values::default();
    let mut keys = BTreeSet::new();
    let mut items: BTreeMap<String, Json> = BTreeMap::new();
    let mut labelled = BTreeSet::new();
    let mut replicates: BTreeMap<(String, String, String), BTreeSet<u64>> = BTreeMap::new();
    for (_, r) in records {
        match r["record"].as_str() {
            Some("item") => {
                let id = text(&r["item_id"]);
                match items.get(&id) {
                    Some(prev) if prev != r => integrity.conflicting_items += 1,
                    Some(_) => {}
                    None => {
                        items.insert(id, r.clone());
                    }
                }
            }
            Some("label") => {
                labelled.insert(text(&r["item_id"]));
            }
            Some("judgment") => {
                let task = r["task"].as_str().unwrap_or("default").to_owned();
                let key = json!([
                    r["item_id"],
                    r["run_id"],
                    task,
                    r["gen_replicate"],
                    r["grade_replicate"]
                ])
                .to_string();
                if !keys.insert(key) {
                    integrity.duplicate_judgments += 1;
                }
                if let Some(g) = r["gen_replicate"].as_u64() {
                    replicates
                        .entry((text(&r["item_id"]), text(&r["run_id"]), task))
                        .or_default()
                        .insert(g);
                }
                match (&r["value"], &r["missing"]) {
                    (v, _) if !v.is_null() => *values.judgments.entry(text(v)).or_insert(0) += 1,
                    (_, m) => {
                        *values.missing.entry(text(&m["reason"])).or_insert(0) += 1;
                        if let Some(raw) = m["detail"]
                            .as_str()
                            .and_then(|d| d.strip_suffix(" is not mapped"))
                            .and_then(|d| {
                                d.split_once(": ")
                                    .map(|(_, v)| v.trim_matches('"').to_owned())
                            })
                        {
                            *values.not_mapped.entry(raw).or_insert(0) += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    integrity.replicate_gaps = replicates
        .values()
        .filter(|g| g.last().is_some_and(|max| *max + 1 != g.len() as u64))
        .count();
    if maps_labels {
        integrity.items_without_label =
            Some(items.keys().filter(|i| !labelled.contains(*i)).count());
    }
    Report {
        coverage,
        unused_paths: unused(rows, mapping_text),
        integrity,
        values,
    }
}

fn sorted(mut v: Vec<Json>) -> Vec<Json> {
    v.sort_by_key(|x| x.to_string());
    v
}

/// Check `mapping.tests.jsonl` lines `{"input": <row>, "records": [...]}`: the mapping
/// must give exactly these records for the row.
pub fn cases(c: &Compiled, tests: &str, host: &mut dyn Host) -> Result<Cases, MapError> {
    let mut out = Cases::default();
    for (i, line) in tests
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let case: Json = serde_json::from_str(line)
            .map_err(|e| MapError::Read(format!("tests line {}: {e}", i + 1)))?;
        let expected = sorted(case["records"].as_array().cloned().unwrap_or_default());
        let (got, _) = mapping::apply(c, &[(1, case["input"].clone())], host)?;
        let got = sorted(got.into_iter().map(|(_, r)| r).collect());
        out.cases += 1;
        if got == expected {
            out.passed += 1;
        } else {
            out.failures.push(CaseFailure {
                line: i + 1,
                expected,
                got,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_paths_cover_templates_insertions_and_filters() {
        let text = r#"id = { template = "{$.task}:{$.ticket_id}" }
value = { path = "$.runs.{r}.level" }
features = { explode = "$.runs.{r}.rules[?(@.applies)]", feature_id = "@.rule_id" }"#;
        let (rooted, relative) = written_paths(text);
        assert!(rooted.contains(&vec!["task".to_owned()]));
        assert!(rooted.contains(&vec!["runs".into(), "*".into(), "level".into()]));
        assert!(rooted.contains(&vec!["runs".into(), "*".into(), "rules".into()]));
        assert!(relative.contains("rule_id") && relative.contains("applies"));
    }
}
