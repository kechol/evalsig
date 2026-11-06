//! Content bodies and the free-text fields of records under `store.content`, and
//! their removal by `purge` (D-024).
//!
//! A record's `*_ref` is always the hash of the original body, so the storage mode
//! changes what `objects/` holds, never a snapshot's ID.

use std::borrow::Cow;
use std::collections::BTreeSet;

use evalsig_core::config::ContentMode;
use evalsig_core::records::{Content, Missing, Record, Stored};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::canonical::CanonicalError;
use crate::hash::ContentHash;
use crate::objects::{ObjectError, ObjectStore};

/// What a pattern match is replaced with.
pub const REDACTED: &str = "[redacted]";

#[derive(Debug, thiserror::Error)]
#[error("redact pattern {pattern:?}: {message}")]
pub struct PatternError {
    pub pattern: String,
    pub message: String,
}

/// The compiled `store.redact_patterns`.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    patterns: Vec<Regex>,
}

impl Redactor {
    pub fn new(patterns: &[String]) -> Result<Self, PatternError> {
        let patterns = patterns
            .iter()
            .map(|p| {
                Regex::new(p).map_err(|e| PatternError {
                    pattern: p.clone(),
                    message: e.to_string(),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Redactor { patterns })
    }

    pub fn redact<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let mut out = Cow::Borrowed(text);
        for p in &self.patterns {
            if let Cow::Owned(s) = p.replace_all(&out, REDACTED) {
                out = Cow::Owned(s);
            }
        }
        out
    }

    /// Redact in place; returns how many strings changed.
    fn apply(&self, s: &mut String) -> usize {
        match self.redact(s) {
            Cow::Owned(r) => {
                *s = r;
                1
            }
            Cow::Borrowed(_) => 0,
        }
    }

    fn apply_json(&self, v: &mut Json) -> usize {
        match v {
            Json::String(s) => self.apply(s),
            Json::Array(xs) => xs.iter_mut().map(|x| self.apply_json(x)).sum(),
            Json::Object(m) => m.values_mut().map(|x| self.apply_json(x)).sum(),
            _ => 0,
        }
    }
}

/// A redacted body: the original's name, which records use, and the name of the
/// redacted bytes that `objects/` holds. Kept in the snapshot's provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redaction {
    pub original: ContentHash,
    pub stored: ContentHash,
}

/// A body prepared under the project's mode: its `content` record, the redaction to
/// keep in the provenance (under `redact`), and the bytes to store, if any.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub content: Content,
    pub redaction: Option<Redaction>,
    pub object: Option<Vec<u8>>,
}

/// Prepare a body without writing anything, so an import can decide first.
pub fn prepare_body(
    mode: ContentMode,
    redactor: &Redactor,
    body: &[u8],
    media_type: &str,
) -> Prepared {
    let original = ContentHash::blake3(body);
    let (stored, redaction, object) = match (mode, std::str::from_utf8(body)) {
        (ContentMode::Full, _) => (Stored::Full, None, Some(body.to_vec())),
        (ContentMode::Redact, Ok(text)) => {
            let redacted = redactor.redact(text).into_owned().into_bytes();
            let name = ContentHash::blake3(&redacted);
            let redaction = (name != original).then_some(Redaction {
                original,
                stored: name,
            });
            (Stored::Redact, redaction, Some(redacted))
        }
        // Patterns cannot be searched for in bytes that are not text.
        (ContentMode::Hash, _) | (ContentMode::Redact, Err(_)) => (Stored::Hash, None, None),
    };
    Prepared {
        content: Content {
            content_ref: original.to_string(),
            media_type: media_type.to_owned(),
            bytes: Some(body.len() as u64),
            stored: Some(stored),
        },
        redaction,
        object,
    }
}

/// Store a body under the project's mode. Returns its `content` record and, under
/// `redact`, the redaction to keep in the provenance.
pub fn store_body(
    store: &ObjectStore,
    mode: ContentMode,
    redactor: &Redactor,
    body: &[u8],
    media_type: &str,
) -> Result<(Content, Option<Redaction>), ObjectError> {
    let p = prepare_body(mode, redactor, body, media_type);
    if let Some(bytes) = &p.object {
        store.put(bytes)?;
    }
    Ok((p.content, p.redaction))
}

/// The free-text fields of a record: `item.attrs`, strata values, `label.reason`
/// and `missing.detail`, as JSON values or strings.
enum Field<'r> {
    Json(&'r mut Json),
    Text(&'r mut String),
}

fn fields(record: &mut Record) -> Vec<Field<'_>> {
    fn missing(m: &mut Option<Missing>) -> Option<Field<'_>> {
        m.as_mut()?.detail.as_mut().map(Field::Text)
    }
    match record {
        Record::Item(i) => i
            .attrs
            .iter_mut()
            .chain(i.strata.iter_mut())
            .flat_map(|m| m.values_mut())
            .map(Field::Json)
            .collect(),
        Record::Label(l) => l
            .reason
            .as_mut()
            .map(Field::Text)
            .into_iter()
            .chain(missing(&mut l.missing))
            .collect(),
        Record::Judgment(j) => missing(&mut j.missing).into_iter().collect(),
        Record::Preference(p) => missing(&mut p.missing).into_iter().collect(),
        Record::Run(_) | Record::Content(_) => Vec::new(),
    }
}

/// Redact the free-text fields of records; returns how many records changed.
pub fn redact_records(records: &mut [Record], redactor: &Redactor) -> usize {
    records
        .iter_mut()
        .map(|r| {
            fields(r)
                .into_iter()
                .map(|f| match f {
                    Field::Json(v) => redactor.apply_json(v),
                    Field::Text(s) => redactor.apply(s),
                })
                .sum::<usize>()
        })
        .filter(|&n| n > 0)
        .count()
}

fn item_id(record: &Record) -> Option<&str> {
    match record {
        Record::Item(i) => Some(&i.item_id),
        Record::Label(l) => Some(&l.item_id),
        Record::Judgment(j) => Some(&j.item_id),
        Record::Preference(p) => Some(&p.item_id),
        Record::Run(_) | Record::Content(_) => None,
    }
}

/// The bodies a record names: its `*_ref` fields, or a `content` record's `ref`.
pub fn bodies(record: &Record) -> impl Iterator<Item = &str> {
    let refs: [Option<&String>; 2] = match record {
        Record::Item(i) => [i.input_ref.as_ref(), i.reference_ref.as_ref()],
        Record::Judgment(j) => [j.output_ref.as_ref(), j.rationale_ref.as_ref()],
        Record::Preference(p) => [p.rationale_ref.as_ref(), None],
        Record::Content(c) => [Some(&c.content_ref), None],
        Record::Label(_) | Record::Run(_) => [None, None],
    };
    refs.into_iter().flatten().map(String::as_str)
}

/// Whether a record other than a `content` record names the body.
fn names(record: &Record, body: &str) -> bool {
    !matches!(record, Record::Content(_)) && bodies(record).any(|r| r == body)
}

/// The redacted copies of `body` that go when it is purged: those no other redaction
/// in force stores and that are not themselves a body some record names (`named`).
pub fn unshared_copies<'r>(
    redactions: impl Iterator<Item = &'r Redaction> + Clone,
    named: &BTreeSet<&str>,
    body: &ContentHash,
) -> Vec<ContentHash> {
    let shared: BTreeSet<ContentHash> = redactions
        .clone()
        .filter(|r| r.original != *body)
        .map(|r| r.stored)
        .collect();
    redactions
        .filter(|r| r.original == *body)
        .map(|r| r.stored)
        .filter(|s| !shared.contains(s) && !named.contains(s.to_string().as_str()))
        .collect()
}

/// The items whose records name the body.
pub fn items_naming(records: &[Record], body: &str) -> BTreeSet<String> {
    records
        .iter()
        .filter(|r| names(r, body))
        .filter_map(item_id)
        .map(str::to_owned)
        .collect()
}

/// The reference that replaces a purged value: BLAKE3 of its canonical JSON.
fn purged(v: &Json) -> Result<String, CanonicalError> {
    Ok(crate::canonical::hash(v)?.to_string())
}

fn is_ref(v: &str) -> bool {
    v.parse::<ContentHash>().is_ok()
}

/// Replace the free-text fields of every record of `items` with references, and mark
/// the body's `content` record purged; returns how many records changed. Values that
/// are already references are kept, so purging twice changes nothing more.
pub fn purge_records(
    records: &mut [Record],
    items: &BTreeSet<String>,
    body: &str,
) -> Result<usize, CanonicalError> {
    let mut changed = 0;
    for r in records.iter_mut() {
        let mut touched = false;
        if let Record::Content(c) = r {
            if c.content_ref == body && c.stored != Some(Stored::Purged) {
                c.stored = Some(Stored::Purged);
                touched = true;
            }
        } else if item_id(r).is_some_and(|i| items.contains(i)) {
            for f in fields(r) {
                match f {
                    Field::Json(v) if !v.as_str().is_some_and(is_ref) => {
                        *v = Json::String(purged(v)?);
                        touched = true;
                    }
                    Field::Text(s) if !is_ref(s) => {
                        *s = purged(&Json::String(std::mem::take(s)))?;
                        touched = true;
                    }
                    _ => {}
                }
            }
        }
        changed += usize::from(touched);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(lines: &str) -> Vec<Record> {
        lines
            .lines()
            .map(|l| Record::from_json(l).unwrap())
            .collect()
    }

    const BODY: &str = "blake3:1111111111111111111111111111111111111111111111111111111111111111";

    fn sample() -> Vec<Record> {
        records(&format!(
            r#"{{"record":"item","item_id":"a","input_ref":"{BODY}","attrs":{{"note":"call Jane on 555-0100","n":3}},"strata":{{"channel":"chat"}}}}
{{"record":"item","item_id":"b","attrs":{{"note":"keep me"}}}}
{{"record":"label","item_id":"a","rater_id":"r","source":"human","value":true,"version":1,"reason":"Jane said so"}}
{{"record":"judgment","item_id":"a","run_id":"x","gen_replicate":0,"output_type":"binary","missing":{{"reason":"error","detail":"Jane timed out"}}}}
{{"record":"content","ref":"{BODY}","media_type":"text/plain","stored":"full"}}"#
        ))
    }

    #[test]
    fn redaction_covers_free_text_fields() {
        let r = Redactor::new(&["(?i)jane".into(), r"\d{3}-\d{4}".into()]).unwrap();
        let mut rs = sample();
        assert_eq!(redact_records(&mut rs, &r), 3);
        let text: String = rs
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect();
        assert!(!text.contains("Jane") && !text.contains("555-0100"));
        assert!(text.contains("call [redacted] on [redacted]"));
        assert!(text.contains("keep me"));
        assert!(Redactor::new(&["(".into()]).is_err());
    }

    #[test]
    fn bodies_follow_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(dir.path());
        let r = Redactor::new(&["secret-\\w+".into()]).unwrap();
        let body = b"token secret-abc here";
        let original = ContentHash::blake3(body);

        let (c, red) = store_body(&store, ContentMode::Hash, &r, body, "text/plain").unwrap();
        assert_eq!(
            (c.content_ref.as_str(), c.stored, red),
            (original.to_string().as_str(), Some(Stored::Hash), None)
        );
        assert!(!store.contains(&original));

        let (c, red) = store_body(&store, ContentMode::Redact, &r, body, "text/plain").unwrap();
        let red = red.unwrap();
        assert_eq!(c.content_ref, original.to_string());
        assert_eq!(red.original, original);
        assert_eq!(store.get(&red.stored).unwrap(), b"token [redacted] here");
        assert!(!store.contains(&original));

        let (c, _) = store_body(&store, ContentMode::Full, &r, body, "text/plain").unwrap();
        assert_eq!(c.stored, Some(Stored::Full));
        assert_eq!(store.get(&original).unwrap(), body);
    }

    #[test]
    fn copies_another_body_uses_are_kept() {
        let h = |b: &[u8]| ContentHash::blake3(b);
        let (x, y, z) = (h(b"x"), h(b"y"), h(b"z"));
        let rs = [
            Redaction {
                original: x,
                stored: y,
            },
            Redaction {
                original: x,
                stored: z,
            },
        ];
        let named = BTreeSet::from([y.to_string()]);
        let named: BTreeSet<&str> = named.iter().map(String::as_str).collect();
        assert_eq!(unshared_copies(rs.iter(), &named, &x), [z]);
        let shared = [
            rs[1].clone(),
            Redaction {
                original: y,
                stored: z,
            },
        ];
        assert!(unshared_copies(shared.iter(), &BTreeSet::new(), &x).is_empty());
    }

    #[test]
    fn purge_replaces_the_items_text_once() {
        let mut rs = sample();
        let items = items_naming(&rs, BODY);
        assert_eq!(items, BTreeSet::from(["a".to_owned()]));
        assert_eq!(purge_records(&mut rs, &items, BODY).unwrap(), 4);
        let text: String = rs
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect();
        assert!(!text.contains("Jane") && !text.contains("chat"));
        assert!(text.contains("keep me") && text.contains(r#""stored":"purged""#));
        assert_eq!(purge_records(&mut rs, &items, BODY).unwrap(), 0);
    }
}
