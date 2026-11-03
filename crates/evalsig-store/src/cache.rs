//! The SQLite working copy under `.evalsig/cache/` (D-006).
//!
//! It is derived data: rebuilt from the canonical snapshots at any time and never
//! the basis of a hash. Tables are STRICT, triggers refuse UPDATE and DELETE (rows
//! change only by a rebuild), and WAL lets readers such as the dashboard read while
//! the CLI writes.

use std::path::Path;

use evalsig_core::records::Record;
use rusqlite::{Connection, params};

use crate::hash::ContentHash;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
CREATE TABLE snapshots (seq INTEGER PRIMARY KEY, hash TEXT NOT NULL) STRICT;
CREATE TABLE runs (
  run_id TEXT PRIMARY KEY, system_id TEXT NOT NULL, system_version TEXT NOT NULL,
  value_semantics TEXT NOT NULL, output_type TEXT, levels INTEGER, record TEXT NOT NULL
) STRICT;
CREATE TABLE items (
  item_id TEXT PRIMARY KEY, cluster TEXT NOT NULL, weight REAL NOT NULL,
  occurred_at TEXT, record TEXT NOT NULL
) STRICT;
CREATE TABLE labels (
  item_id TEXT NOT NULL, rater_id TEXT NOT NULL, task TEXT NOT NULL,
  label_replicate INTEGER NOT NULL, version INTEGER NOT NULL, source TEXT NOT NULL,
  value REAL, missing TEXT, record TEXT NOT NULL,
  PRIMARY KEY (item_id, rater_id, task, label_replicate, version)
) STRICT;
CREATE TABLE judgments (
  item_id TEXT NOT NULL, run_id TEXT NOT NULL, task TEXT NOT NULL,
  gen_replicate INTEGER NOT NULL, grade_replicate INTEGER NOT NULL,
  value REAL, missing TEXT, record TEXT NOT NULL,
  PRIMARY KEY (item_id, run_id, task, gen_replicate, grade_replicate)
) STRICT;
CREATE TABLE preferences (id INTEGER PRIMARY KEY, item_id TEXT NOT NULL, record TEXT NOT NULL) STRICT;
CREATE TABLE contents (ref TEXT PRIMARY KEY, record TEXT NOT NULL) STRICT;
"#;

const TABLES: [&str; 7] = [
    "snapshots",
    "runs",
    "items",
    "labels",
    "judgments",
    "preferences",
    "contents",
];

fn append_only_triggers() -> String {
    let mut sql = String::new();
    for t in TABLES {
        for (op, name) in [("UPDATE", "update"), ("DELETE", "delete")] {
            sql.push_str(&format!(
                "CREATE TRIGGER {t}_no_{name} BEFORE {op} ON {t} \
                 BEGIN SELECT RAISE(ABORT, 'the working copy is rebuilt, never edited'); END;\n"
            ));
        }
    }
    sql
}

fn value_of(r: &Record) -> Option<f64> {
    match r {
        Record::Judgment(j) => j.value.map(|v| v.as_f64()),
        Record::Label(l) => l.value.map(|v| v.as_f64()),
        _ => None,
    }
}

/// Replace the working copy at `path` with one built from `records` (the layered
/// project data) and the snapshots they came from.
pub fn rebuild(
    path: &Path,
    records: &[Record],
    snapshots: &[ContentHash],
) -> Result<(), CacheError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    for suffix in ["", "-wal", "-shm"] {
        let p = path.with_file_name(format!(
            "{}{suffix}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("evalsig.db")
        ));
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    let mut conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(&append_only_triggers())?;
    let tx = conn.transaction()?;
    for (i, h) in snapshots.iter().enumerate() {
        tx.execute(
            "INSERT INTO snapshots VALUES (?1, ?2)",
            params![i as i64, h.to_string()],
        )?;
    }
    for r in records {
        let json = serde_json::to_string(r).expect("records serialize");
        let missing = r.missing().map(|m| {
            serde_json::to_value(m.reason)
                .expect("reason serializes")
                .as_str()
                .unwrap_or("")
                .to_owned()
        });
        match r {
            Record::Run(x) => tx.execute(
                "INSERT INTO runs VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    x.run_id,
                    x.system.id,
                    x.system.version,
                    serde_json::to_value(x.value_semantics())
                        .expect("serializes")
                        .as_str(),
                    x.output_type.map(|t| serde_json::to_value(t)
                        .expect("serializes")
                        .as_str()
                        .unwrap_or("")
                        .to_owned()),
                    x.levels,
                    json
                ],
            )?,
            Record::Item(x) => tx.execute(
                "INSERT INTO items VALUES (?1, ?2, ?3, ?4, ?5)",
                params![x.item_id, x.cluster_key(), x.weight(), x.occurred_at, json],
            )?,
            Record::Label(x) => tx.execute(
                "INSERT INTO labels VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    x.item_id,
                    x.rater_id,
                    x.task(),
                    x.label_replicate(),
                    x.version,
                    serde_json::to_value(x.source).expect("serializes").as_str(),
                    value_of(r),
                    missing,
                    json
                ],
            )?,
            Record::Judgment(x) => tx.execute(
                "INSERT INTO judgments VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    x.item_id,
                    x.run_id,
                    x.task(),
                    x.gen_replicate,
                    x.grade_replicate(),
                    value_of(r),
                    missing,
                    json
                ],
            )?,
            Record::Preference(x) => tx.execute(
                "INSERT INTO preferences (item_id, record) VALUES (?1, ?2)",
                params![x.item_id, json],
            )?,
            Record::Content(x) => tx.execute(
                "INSERT INTO contents VALUES (?1, ?2)",
                params![x.content_ref, json],
            )?,
        };
    }
    tx.commit()?;
    Ok(())
}

/// Open an existing working copy for reading.
pub fn open(path: &Path) -> Result<Connection, CacheError> {
    Ok(Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

/// Row counts per table, in table order.
pub fn counts(conn: &Connection) -> Result<Vec<(&'static str, i64)>, CacheError> {
    TABLES
        .iter()
        .map(|t| {
            Ok((
                *t,
                conn.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))?,
            ))
        })
        .collect()
}

/// Every row of every table in primary-key order, as text: two working copies built
/// from the same data give the same dump.
pub fn dump(conn: &Connection) -> Result<String, CacheError> {
    let mut out = String::new();
    for t in TABLES {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {t} ORDER BY 1, 2"))?;
        let n = stmt.column_count();
        let rows = stmt.query_map([], |row| {
            (0..n)
                .map(|i| row.get_ref(i).map(|v| format!("{v:?}")))
                .collect::<Result<Vec<_>, _>>()
        })?;
        for r in rows {
            out.push_str(t);
            out.push('\t');
            out.push_str(&r?.join("\t"));
            out.push('\n');
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records() -> Vec<Record> {
        include_str!("../../../examples/records.jsonl")
            .lines()
            .map(|l| Record::from_json(l).unwrap())
            .collect()
    }

    #[test]
    fn rows_cannot_be_edited() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cache/evalsig.db");
        rebuild(&db, &records(), &[]).unwrap();
        let conn = Connection::open(&db).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        for sql in [
            "UPDATE items SET weight = 3",
            "DELETE FROM judgments",
            "UPDATE runs SET system_version = 'x'",
        ] {
            let err = conn.execute(sql, []).unwrap_err().to_string();
            assert!(err.contains("rebuilt, never edited"), "{sql}: {err}");
        }
        assert!(
            conn.execute(
                "INSERT INTO items VALUES ('x', 'x', 1.0, NULL, 'not strict' )",
                []
            )
            .is_ok()
        );
        assert!(
            conn.execute(
                "INSERT INTO items VALUES ('y', 'y', 'heavy', NULL, '{}')",
                []
            )
            .is_err(),
            "STRICT types"
        );
    }

    #[test]
    fn counts_match_the_records() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("evalsig.db");
        rebuild(&db, &records(), &[]).unwrap();
        let counts = counts(&open(&db).unwrap()).unwrap();
        assert_eq!(
            counts,
            vec![
                ("snapshots", 0),
                ("runs", 3),
                ("items", 3),
                ("labels", 8),
                ("judgments", 7),
                ("preferences", 3),
                ("contents", 1)
            ]
        );
    }
}
