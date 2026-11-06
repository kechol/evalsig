//! The project's imported data: the snapshots named by `import` ledger lines (as
//! replaced by `purge` and `migrate` lines), layered in import order so a later
//! import of the same primary key wins.

use std::collections::BTreeMap;

use evalsig_core::labels::LabelSet;
use evalsig_core::records::{Item, Judgment, Label, Record, Run};
use evalsig_core::time::Instant;
use evalsig_store::hash::ContentHash;
use evalsig_store::objects::ObjectStore;
use evalsig_store::rows;
use evalsig_store::snapshot::{self, InForce, Provenance};

use crate::Error;
use crate::exit::Exit;
use crate::project::Project;

/// The name of a set of records: the BLAKE3 reference of their canonical lines,
/// sorted and concatenated, so the order of a file does not matter.
pub fn records_hash<'a>(records: impl Iterator<Item = &'a Record>) -> Result<String, Error> {
    let mut lines = records
        .map(|r| {
            evalsig_store::canonical::to_canonical_bytes(
                &serde_json::to_value(r).expect("records serialize"),
            )
            .map_err(Error::store)
        })
        .collect::<Result<Vec<_>, _>>()?;
    lines.sort();
    Ok(ContentHash::blake3(&lines.concat()).to_string())
}

pub struct Data {
    pub records: Vec<Record>,
    pub snapshots: Vec<ContentHash>,
}

pub fn load(project: &Project) -> Result<Data, Error> {
    let snapshots = in_force(project)?.into_iter().map(|x| x.snapshot).collect();
    load_snapshots(project, snapshots)
}

/// The snapshots in force, as the project's ledger names them.
pub fn in_force(project: &Project) -> Result<Vec<InForce>, Error> {
    let entries = project.entries()?;
    snapshot::in_force(&entries).map_err(Error::store)
}

/// The provenance object of a snapshot in force.
pub fn provenance_of(store: &ObjectStore, x: &InForce) -> Result<Provenance, Error> {
    let p = x
        .provenance
        .ok_or_else(|| Error::store(format!("snapshot {} has no provenance", x.snapshot)))?;
    snapshot::read_provenance(store, &p).map_err(Error::store)
}

/// The data of exactly these snapshots, layered in the given order.
pub fn load_snapshots(project: &Project, snapshots: Vec<ContentHash>) -> Result<Data, Error> {
    let store = project.objects();
    let read = snapshots
        .iter()
        .map(|h| snapshot::read(&store, h).map_err(Error::store))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(layered(snapshots, read))
}

/// Data from snapshots already read, in the same order.
pub fn layered(snapshots: Vec<ContentHash>, read: Vec<Vec<Record>>) -> Data {
    Data {
        records: rows::layer(read),
        snapshots,
    }
}

impl Data {
    /// The snapshot references, as provenance records them.
    pub fn snapshot_ids(&self) -> Vec<String> {
        self.snapshots.iter().map(ToString::to_string).collect()
    }

    pub fn run(&self, id: &str) -> Result<&Run, Error> {
        self.records
            .iter()
            .find_map(|r| match r {
                Record::Run(run) if run.run_id == id => Some(run),
                _ => None,
            })
            .ok_or_else(|| {
                Error::usage(format!("no run named {id:?}"))
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
    pub fn labels(&self, as_of: Option<&str>) -> Result<(Vec<Label>, usize), Error> {
        let mut set = LabelSet::default();
        for r in &self.records {
            if let Record::Label(l) = r {
                set.insert(l.clone())
                    .map_err(|e| Error::new("invalid_labels", e.to_string(), Exit::Blocked))?;
            }
        }
        Ok(match as_of {
            None => (set.latest().into_iter().cloned().collect(), 0),
            Some(t) => {
                let at = Instant::parse(t).map_err(|e| {
                    Error::usage(format!("--labels-as-of: {e}"))
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
