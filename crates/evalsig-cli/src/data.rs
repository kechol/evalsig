//! The project's imported data: the snapshots named by `import` ledger lines,
//! layered in import order so a later import of the same primary key wins.

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
    let mut snapshots: Vec<ContentHash> = Vec::new();
    for e in entries.iter().filter(|e| e.kind == Kind::Import) {
        let h: ContentHash = e
            .outputs
            .first()
            .and_then(|o| o.parse().ok())
            .ok_or_else(|| CliError::store(format!("import line {} names no snapshot", e.seq)))?;
        snapshots.retain(|s| *s != h);
        snapshots.push(h);
    }
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
