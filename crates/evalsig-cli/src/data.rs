//! The project's imported data: the snapshots named by `import` ledger lines,
//! layered in import order so a later import of the same primary key wins.

use evalsig_core::records::Record;
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
    let entries = ledger::read(&project.ledger_path())
        .map_err(|e| CliError::new("ledger", e.to_string(), Exit::Blocked))?;
    let mut snapshots: Vec<ContentHash> = Vec::new();
    for e in entries.iter().filter(|e| e.kind == Kind::Import) {
        let Some(first) = e.outputs.first() else {
            continue;
        };
        let h: ContentHash = first.parse().map_err(|_| {
            CliError::new(
                "ledger",
                format!("import line {} names no snapshot", e.seq),
                Exit::Blocked,
            )
        })?;
        snapshots.retain(|s| *s != h);
        snapshots.push(h);
    }
    let store = project.objects();
    let mut layered: Vec<(usize, Record)> = Vec::new();
    for (i, h) in snapshots.iter().enumerate() {
        let rs = snapshot::read(&store, h)
            .map_err(|e| CliError::new("store", e.to_string(), Exit::Blocked))?;
        layered.extend(rs.into_iter().map(|r| (i, r)));
    }
    layered.sort_by(|a, b| rows::compare(&a.1, &b.1).then(a.0.cmp(&b.0)));
    let mut records: Vec<Record> = Vec::with_capacity(layered.len());
    for (_, r) in layered {
        match records.last_mut() {
            Some(last) if rows::compare(last, &r).is_eq() => *last = r,
            _ => records.push(r),
        }
    }
    Ok(Data { records, snapshots })
}
