//! `evalsig rebuild`: recreate the SQLite working copy from the canonical snapshots.

use std::path::Path;

use evalsig_store::cache;
use serde_json::json;

use crate::data;
use crate::error::CliError;
use crate::output::{Envelope, Output};
use crate::project::Project;

pub fn run(root: &Path) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let d = data::load(&project)?;
    let db = project.cache_path();
    cache::rebuild(&db, &d.records, &d.snapshots).map_err(CliError::store)?;
    let conn = cache::open(&db).map_err(CliError::store)?;
    let counts: serde_json::Map<String, serde_json::Value> = cache::counts(&conn)
        .map_err(CliError::store)?
        .into_iter()
        .map(|(t, n)| (t.to_owned(), json!(n)))
        .collect();
    let mut envelope = Envelope::new(
        "evalsig.rebuild/1",
        json!({ "cache": db.display().to_string(), "rows": counts }),
    );
    envelope.provenance.snapshots = d.snapshots.iter().map(ToString::to_string).collect();
    Ok(Output::success(envelope))
}
