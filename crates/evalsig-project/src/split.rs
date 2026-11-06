//! The agent-side splits recorded in the project ledger (SPEC §8.1, D-022), as the
//! analyses read them.

use std::collections::BTreeSet;

use evalsig_core::split::{Part, SplitError};
use evalsig_store::ledger::{Entry, Kind};
use serde_json::Value;

use crate::Error;
use crate::project::{Project, hex, io_error, str_set};

/// The recorded agent-side splits, oldest first.
pub fn split_entries(project: &Project) -> Result<Vec<Entry>, Error> {
    Ok(project
        .entries()?
        .into_iter()
        .filter(|e| e.kind == Kind::Split)
        .collect())
}

/// A part of a recorded split: its hash and items.
#[derive(Debug, Clone)]
pub struct Scope {
    pub hash: String,
    pub items: BTreeSet<String>,
}

/// One part of the latest recorded split, or of the split whose part has `hash`.
pub fn scope(project: &Project, part: Part, hash: Option<&str>) -> Result<Scope, Error> {
    let key = match part {
        Part::Dev => "dev",
        Part::Validation => "validation",
    };
    let part_hash = |e: &Entry| {
        e.summary
            .get(key)
            .and_then(|p| p["hash"].as_str())
            .map(str::to_owned)
    };
    let entry = split_entries(project)?
        .into_iter()
        .rev()
        .find(|e| hash.is_none_or(|h| part_hash(e).as_deref() == Some(h)))
        .ok_or_else(|| {
            Error::usage(match hash {
                Some(h) => format!("no recorded split has a {key} part {h}"),
                None => "no split is recorded".to_owned(),
            })
            .with_hint("run `evalsig split` first")
        })?;
    let name = entry.outputs.first().cloned().unwrap_or_default();
    let path = project.splits_dir().join(format!("{}.json", hex(&name)));
    let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, &e))?;
    let file: Value = serde_json::from_str(&text)
        .map_err(|e| Error::usage(format!("{}: {e}", path.display())))?;
    Ok(Scope {
        hash: part_hash(&entry).unwrap_or_default(),
        items: str_set(&file[key]),
    })
}

/// The same mistake gets the same code and exit on either side of the split.
pub fn split_error(e: SplitError) -> Error {
    match e {
        SplitError::CrossesCluster { .. } => {
            Error::rejected("split_crosses_cluster", e.to_string())
        }
        SplitError::NotAvailable(_) => Error::rejected("holdout_item", e.to_string()),
        SplitError::Share => Error::usage(e.to_string()),
        _ => Error::rejected("split", e.to_string()),
    }
}
