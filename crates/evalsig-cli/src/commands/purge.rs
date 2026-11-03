//! `evalsig purge <ref>`: remove a body and the free-text fields of the items that
//! name it, keeping the ledger verifiable (D-024).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use evalsig_store::content;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind};
use evalsig_store::{cache, snapshot};
use serde_json::{Value, json};

use crate::data;
use crate::error::CliError;
use crate::output::{Envelope, Output};
use crate::project::{Project, io_error};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The body to remove, as its `blake3:` or `sha256:` reference.
    pub body: String,
    /// Why the body is removed; kept in the ledger.
    #[arg(long)]
    pub reason: String,
    /// Confirm the removal, which cannot be undone.
    #[arg(long)]
    pub yes: bool,
}

/// Saved reports computed from any of these snapshots.
fn reports_using(dir: &Path, snapshots: &BTreeSet<String>) -> Result<Vec<PathBuf>, CliError> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(out);
    };
    for entry in entries {
        let path = entry.map_err(|e| io_error(dir, &e))?.path();
        let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, &e))?;
        let uses = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["provenance"]["snapshots"].as_array().cloned())
            .into_iter()
            .flatten()
            .any(|s| s.as_str().is_some_and(|s| snapshots.contains(s)));
        if uses {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let body: ContentHash = args.body.parse().map_err(|_| {
        CliError::usage(format!("{:?} is not a content reference", args.body))
            .with_hint("pass blake3:<64 hex> or sha256:<64 hex>")
    })?;
    if !args.yes {
        return Err(
            CliError::usage("purge deletes content and cannot be undone")
                .with_hint("pass --yes to confirm"),
        );
    }
    let body_ref = body.to_string();
    let project = Project::open(root)?;
    let store = project.objects();
    let entries = ledger::read(&project.ledger_path()).map_err(CliError::store)?;
    let current = data::current(&entries)?;
    let mut sets = Vec::new();
    for (s, p) in &current {
        let records = snapshot::read(&store, s).map_err(CliError::store)?;
        let p = p.ok_or_else(|| CliError::store(format!("snapshot {s} has no provenance")))?;
        let prov = snapshot::read_provenance(&store, &p).map_err(CliError::store)?;
        sets.push((*s, records, prov));
    }
    let items: BTreeSet<String> = sets
        .iter()
        .flat_map(|(_, rs, _)| content::items_naming(rs, &body_ref))
        .collect();
    let redactions = || sets.iter().flat_map(|(_, _, p)| &p.redactions);
    let mut doomed: Vec<ContentHash> = redactions()
        .filter(|r| r.original == body)
        .map(|r| r.stored)
        // A redacted copy shared with another body stays.
        .filter(|s| !redactions().any(|r| r.original != body && r.stored == *s))
        .chain(store.contains(&body).then_some(body))
        .collect();
    if items.is_empty() && doomed.is_empty() {
        return Err(CliError::usage(format!(
            "no record names {body_ref} and it is not stored"
        )));
    }
    let mut replaced = Vec::new();
    let mut records_changed = 0;
    for (old, mut records, prov) in sets {
        let n = content::purge_records(&mut records, &items, &body_ref).map_err(CliError::store)?;
        if n == 0 {
            continue;
        }
        records_changed += n;
        let written = snapshot::write(&store, records, |snapshot| {
            let mut p = prov;
            p.snapshot = snapshot;
            p.redactions.retain(|r| r.original != body);
            p
        })
        .map_err(CliError::store)?;
        replaced.push(json!({
            "from": old,
            "to": written.snapshot,
            "provenance": written.provenance,
        }));
        doomed.push(old);
    }
    let superseded: BTreeSet<String> = replaced
        .iter()
        .filter_map(|r| r["from"].as_str().map(str::to_owned))
        .collect();
    let reports = reports_using(&project.dir.join("reports"), &superseded)?;
    let report_names: Vec<String> = reports
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let mut envelope = Envelope::new(
        "evalsig.purge/1",
        json!({
            "body": body_ref,
            "items": items,
            "records_changed": records_changed,
            "replaced": replaced,
            "deleted_objects": doomed,
            "deleted_reports": report_names,
        }),
    );
    let summary = json!({
        "reason": args.reason,
        "replaced": replaced,
        "records_changed": records_changed,
        "deleted_objects": doomed,
        "deleted_reports": report_names,
    });
    let outputs = replaced
        .iter()
        .flat_map(|r| [&r["to"], &r["provenance"]])
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    // The ledger names the replacements before anything is deleted, so an
    // interrupted purge leaves text behind rather than data that cannot be loaded.
    project.record(
        &mut envelope,
        Kind::Purge,
        std::iter::once(body_ref)
            .chain(superseded.iter().cloned())
            .collect(),
        outputs,
        summary.as_object().cloned().unwrap_or_default(),
    )?;
    for h in &doomed {
        store.remove(h).map_err(CliError::store)?;
    }
    for p in &reports {
        std::fs::remove_file(p).map_err(|e| io_error(p, &e))?;
    }
    let db = project.cache_path();
    if db.exists() {
        let d = data::load(&project)?;
        cache::rebuild(&db, &d.records, &d.snapshots).map_err(CliError::store)?;
    }
    Ok(Output::success(envelope))
}
