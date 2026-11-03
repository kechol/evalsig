//! The project's imported data: the snapshots named by `import` ledger lines (as
//! replaced by `purge` and `migrate` lines), layered in import order so a later
//! import of the same primary key wins.

use std::collections::BTreeMap;

use evalsig_core::labels::LabelSet;
use evalsig_core::records::{Item, Judgment, Label, Record, Run};
use evalsig_core::time::Instant;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind};
use evalsig_store::{rows, snapshot};

use crate::error::CliError;
use crate::exit::Exit;
use crate::project::Project;

pub struct Data {
    pub records: Vec<Record>,
    pub snapshots: Vec<ContentHash>,
}

pub fn load(project: &Project) -> Result<Data, CliError> {
    let entries = ledger::read(&project.ledger_path()).map_err(CliError::store)?;
    let snapshots = current(&entries)?.into_iter().map(|(s, _)| s).collect();
    load_snapshots(project, snapshots)
}

fn hash(v: Option<&str>, e: &ledger::Entry) -> Result<ContentHash, CliError> {
    v.and_then(|o| o.parse().ok())
        .ok_or_else(|| CliError::store(format!("ledger line {} names no snapshot", e.seq)))
}

/// The snapshots in force with their provenance objects: those named by `import`
/// lines in import order, each once (its last import), and in place of each one a
/// `purge` or `migrate` line replaced, the replacement.
pub fn current(
    entries: &[ledger::Entry],
) -> Result<Vec<(ContentHash, Option<ContentHash>)>, CliError> {
    let mut snapshots: Vec<(ContentHash, Option<ContentHash>)> = Vec::new();
    for e in entries {
        match e.kind {
            Kind::Import => {
                let s = hash(e.outputs.first().map(String::as_str), e)?;
                let p = e.outputs.get(1).and_then(|o| o.parse().ok());
                snapshots.retain(|(x, _)| *x != s);
                snapshots.push((s, p));
            }
            Kind::Purge | Kind::Migrate => {
                let replaced = e.summary.get("replaced").and_then(|v| v.as_array());
                for r in replaced.into_iter().flatten() {
                    let from = hash(r["from"].as_str(), e)?;
                    let to = hash(r["to"].as_str(), e)?;
                    let p = r["provenance"].as_str().and_then(|o| o.parse().ok());
                    for slot in snapshots.iter_mut().filter(|(x, _)| *x == from) {
                        *slot = (to, p);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(snapshots)
}

/// The data of exactly these snapshots, layered in the given order.
pub fn load_snapshots(project: &Project, snapshots: Vec<ContentHash>) -> Result<Data, CliError> {
    let store = project.objects();
    let read = snapshots
        .iter()
        .map(|h| snapshot::read(&store, h).map_err(CliError::store))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Data {
        records: rows::layer(read),
        snapshots,
    })
}

impl Data {
    pub fn run(&self, id: &str) -> Result<&Run, CliError> {
        self.records
            .iter()
            .find_map(|r| match r {
                Record::Run(run) if run.run_id == id => Some(run),
                _ => None,
            })
            .ok_or_else(|| {
                CliError::usage(format!("no run named {id:?}"))
                    .with_hint("list runs with `evalsig runs`")
            })
    }

    pub fn judgments(&self, run_id: &str) -> Vec<&Judgment> {
        self.records
            .iter()
            .filter_map(|r| match r {
                Record::Judgment(j) if j.run_id == run_id => Some(j),
                _ => None,
            })
            .collect()
    }

    pub fn items(&self) -> BTreeMap<&str, &Item> {
        self.records
            .iter()
            .filter_map(|r| match r {
                Record::Item(i) => Some((i.item_id.as_str(), i)),
                _ => None,
            })
            .collect()
    }

    /// The labels in force: the latest revisions, or those as of an RFC 3339 time,
    /// with the count of labels whose newer revisions are undated.
    pub fn labels(&self, as_of: Option<&str>) -> Result<(Vec<Label>, usize), CliError> {
        let mut set = LabelSet::default();
        for r in &self.records {
            if let Record::Label(l) = r {
                set.insert(l.clone())
                    .map_err(|e| CliError::new("invalid_labels", e.to_string(), Exit::Blocked))?;
            }
        }
        Ok(match as_of {
            None => (set.latest().into_iter().cloned().collect(), 0),
            Some(t) => {
                let at = Instant::parse(t).map_err(|e| {
                    CliError::usage(format!("--labels-as-of: {e}"))
                        .with_hint("use RFC 3339, e.g. 2026-09-01T00:00:00Z")
                })?;
                let chosen = set.as_of(at);
                (
                    chosen.labels.into_iter().cloned().collect(),
                    chosen.undated_skipped,
                )
            }
        })
    }
}
