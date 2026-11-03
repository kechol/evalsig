//! Upgrading snapshots written in an earlier canonical record format (SPEC §10.1
//! `migrate`).
//!
//! Record lines carry no format version, so a format is recognised by shape: lines
//! that all read as the current format are current; otherwise the migration whose
//! `detect` accepts every line names the format they are in.

use evalsig_core::records::{RECORDS_VERSION, Record};
use serde_json::Value as Json;

use crate::hash::ContentHash;
use crate::objects::{ObjectError, ObjectStore};

/// One step from a record format to the next.
pub struct Migration {
    pub from: &'static str,
    pub to: &'static str,
    /// Whether a line is in the `from` format.
    pub detect: fn(&Json) -> bool,
    pub apply: fn(Json) -> Result<Json, String>,
}

/// The migrations evalsig ships, oldest first. `records/1` is the first format, so
/// there are none yet.
pub const MIGRATIONS: &[Migration] = &[];

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error(transparent)]
    Object(#[from] ObjectError),
    #[error("line {line}: not JSON: {message}")]
    Json { line: usize, message: String },
    #[error("the lines are in no known record format")]
    Unknown,
    #[error("line {line}: {from} to {to}: {message}")]
    Step {
        line: usize,
        from: &'static str,
        to: &'static str,
        message: String,
    },
    #[error("line {line}: not a {RECORDS_VERSION} record after migration: {message}")]
    Record { line: usize, message: String },
}

/// A snapshot's lines as JSON, without reading them as records.
pub fn read_lines(store: &ObjectStore, hash: &ContentHash) -> Result<Vec<Json>, MigrateError> {
    let bytes = store.get(hash)?;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    bytes
        .split(|b| *b == b'\n')
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_slice(l).map_err(|e| MigrateError::Json {
                line: i + 1,
                message: e.to_string(),
            })
        })
        .collect()
}

/// The format the lines are in.
pub fn detect(lines: &[Json], migrations: &[Migration]) -> Option<&'static str> {
    if lines.iter().all(|l| Record::from_value(l.clone()).is_ok()) {
        return Some(RECORDS_VERSION);
    }
    migrations
        .iter()
        .find(|m| lines.iter().all(m.detect))
        .map(|m| m.from)
}

/// Bring lines to the current format. Returns the records and the formats passed
/// through, starting with the one detected; one entry means the lines were current.
pub fn upgrade(
    mut lines: Vec<Json>,
    migrations: &[Migration],
) -> Result<(Vec<Record>, Vec<&'static str>), MigrateError> {
    let mut path = vec![detect(&lines, migrations).ok_or(MigrateError::Unknown)?];
    while path.last() != Some(&RECORDS_VERSION) {
        // Each step is taken at most once, so a cycle in the table cannot loop.
        if path.len() > migrations.len() {
            return Err(MigrateError::Unknown);
        }
        let from = path[path.len() - 1];
        let m = migrations
            .iter()
            .find(|m| m.from == from)
            .ok_or(MigrateError::Unknown)?;
        lines = lines
            .into_iter()
            .enumerate()
            .map(|(i, l)| {
                (m.apply)(l).map_err(|message| MigrateError::Step {
                    line: i + 1,
                    from: m.from,
                    to: m.to,
                    message,
                })
            })
            .collect::<Result<_, _>>()?;
        path.push(m.to);
    }
    let records = lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            Record::from_value(l).map_err(|e| MigrateError::Record {
                line: i + 1,
                message: e.to_string(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok((records, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fictional `records/0` that named the record kind `type`.
    const RECORDS_0: Migration = Migration {
        from: "records/0",
        to: RECORDS_VERSION,
        detect: |l| l.get("type").is_some() && l.get("record").is_none(),
        apply: |mut l| {
            let m = l.as_object_mut().ok_or("not an object")?;
            let kind = m.remove("type").ok_or("no type")?;
            m.insert("record".into(), kind);
            Ok(l)
        },
    };

    fn lines(text: &str) -> Vec<Json> {
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn current_lines_need_no_migration() {
        let l = lines(r#"{"record":"item","item_id":"a"}"#);
        assert_eq!(detect(&l, &[RECORDS_0]), Some(RECORDS_VERSION));
        assert_eq!(upgrade(l, &[]).unwrap().1, [RECORDS_VERSION]);
    }

    #[test]
    fn records_0_lines_are_upgraded() {
        let l =
            lines("{\"type\":\"item\",\"item_id\":\"a\"}\n{\"type\":\"item\",\"item_id\":\"b\"}");
        assert_eq!(detect(&l, &[RECORDS_0]), Some("records/0"));
        let (records, path) = upgrade(l.clone(), &[RECORDS_0]).unwrap();
        assert_eq!(path, ["records/0", RECORDS_VERSION]);
        assert_eq!(records.len(), 2);
        assert!(matches!(upgrade(l, &[]), Err(MigrateError::Unknown)));
    }

    #[test]
    fn a_broken_step_names_the_line() {
        let bad = Migration {
            apply: |_| Ok(serde_json::json!({"record": "nonsense"})),
            ..RECORDS_0
        };
        let l = lines(r#"{"type":"item","item_id":"a"}"#);
        assert!(matches!(
            upgrade(l, &[bad]),
            Err(MigrateError::Record { line: 1, .. })
        ));
    }
}
