//! Snapshots: the immutable record set produced by one import (audit spec §3).
//!
//! A snapshot's ID is the file hash of its canonical records in primary-key order, so
//! it depends only on the data. Where the data came from (files, adapter, mapping,
//! tool version, time, actor) is a separate provenance object that names the snapshot.

use evalsig_core::records::Record;
use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalError, to_canonical_bytes};
use crate::hash::ContentHash;
use crate::objects::{ObjectError, ObjectStore};
use crate::rows;

pub const PROVENANCE_SCHEMA: &str = "evalsig.provenance/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub path: String,
    pub hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub schema: String,
    pub snapshot: ContentHash,
    pub sources: Vec<SourceFile>,
    pub adapter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_version: Option<String>,
    /// Hash of a `mapping.toml` or converter script used to produce the records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping: Option<ContentHash>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mapping_decisions: Vec<ContentHash>,
    pub tool: String,
    pub imported_at: String,
    pub actor: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error(transparent)]
    Object(#[from] ObjectError),
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    #[error("snapshot {hash}, line {line}: {message}")]
    Record {
        hash: ContentHash,
        line: usize,
        message: String,
    },
}

/// What writing a snapshot stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub snapshot: ContentHash,
    pub provenance: ContentHash,
    /// Objects that did not exist before.
    pub new_objects: usize,
}

/// Store records as a snapshot plus its provenance object.
pub fn write(
    store: &ObjectStore,
    mut records: Vec<Record>,
    provenance: impl FnOnce(ContentHash) -> Provenance,
) -> Result<Written, SnapshotError> {
    let (lines, hash) = rows::canonical_lines(&mut records)?;
    let (stored, new_snapshot) = store.put(&lines.join(&b'\n'))?;
    debug_assert_eq!(stored, hash);
    let prov = provenance(hash);
    let (prov_hash, new_prov) = store.put(&to_canonical_bytes(&prov)?)?;
    Ok(Written {
        snapshot: hash,
        provenance: prov_hash,
        new_objects: usize::from(new_snapshot) + usize::from(new_prov),
    })
}

/// Read a snapshot's records back.
pub fn read(store: &ObjectStore, hash: &ContentHash) -> Result<Vec<Record>, SnapshotError> {
    let bytes = store.get(hash)?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    bytes
        .split(|b| *b == b'\n')
        .enumerate()
        .map(|(i, line)| {
            let err = |message: String| SnapshotError::Record {
                hash: *hash,
                line: i + 1,
                message,
            };
            let text = std::str::from_utf8(line).map_err(|e| err(e.to_string()))?;
            Record::from_json(text).map_err(|e| err(e.to_string()))
        })
        .collect()
}

/// Read a provenance object.
pub fn read_provenance(
    store: &ObjectStore,
    hash: &ContentHash,
) -> Result<Provenance, SnapshotError> {
    let bytes = store.get(hash)?;
    serde_json::from_slice(&bytes).map_err(|e| SnapshotError::Record {
        hash: *hash,
        line: 1,
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLES: &str = include_str!("../../../examples/records.jsonl");

    fn records() -> Vec<Record> {
        EXAMPLES
            .lines()
            .map(|l| Record::from_json(l).unwrap())
            .collect()
    }

    fn prov(source: ContentHash) -> impl FnOnce(ContentHash) -> Provenance {
        move |snapshot| Provenance {
            schema: PROVENANCE_SCHEMA.into(),
            snapshot,
            sources: vec![SourceFile {
                path: "examples/records.jsonl".into(),
                hash: source,
            }],
            adapter: "records".into(),
            adapter_version: None,
            mapping: None,
            mapping_decisions: vec![],
            tool: "evalsig 0.1.0".into(),
            imported_at: "2026-11-03T00:00:00Z".into(),
            actor: "user:test".into(),
        }
    }

    #[test]
    fn importing_the_same_input_twice_yields_one_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(dir.path());
        let src = ContentHash::blake3(EXAMPLES.as_bytes());
        let a = write(&store, records(), prov(src)).unwrap();
        let mut reversed = records();
        reversed.reverse();
        let b = write(&store, reversed, prov(src)).unwrap();
        assert_eq!(a.snapshot, b.snapshot);
        assert_eq!(a.provenance, b.provenance);
        assert_eq!(a.new_objects, 2);
        assert_eq!(b.new_objects, 0);
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn snapshots_read_back_in_primary_key_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(dir.path());
        let w = write(&store, records(), prov(ContentHash::blake3(b"x"))).unwrap();
        let back = read(&store, &w.snapshot).unwrap();
        let mut expected = records();
        rows::sort(&mut expected);
        assert_eq!(back, expected);
        let p = read_provenance(&store, &w.provenance).unwrap();
        assert_eq!(p.snapshot, w.snapshot);
    }

    #[test]
    fn content_blobs_are_objects_referenced_by_hash() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(dir.path());
        let (h, _) = store.put("The answer is 42.".as_bytes()).unwrap();
        assert!(h.to_string().starts_with("blake3:"));
        assert_eq!(store.get(&h).unwrap(), b"The answer is 42.");
    }
}
