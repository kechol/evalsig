//! The project's imported data: the snapshots named by `import` ledger lines,
//! layered in import order so a later import of the same primary key wins.

use evalsig_core::records::Record;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind};
use evalsig_store::{rows, snapshot};

use crate::error::CliError;
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
