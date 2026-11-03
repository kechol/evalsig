//! `evalsig purge <ref>`: remove a body and the free-text fields of the items that
//! name it, keeping the ledger verifiable (D-024).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use evalsig_store::content;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use evalsig_store::snapshot::{self, Replacement};
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

/// Saved reports computed from any of these snapshots. Files that are not JSON
/// results are not reports and are left alone.
fn reports_using(dir: &Path, snapshots: &BTreeSet<String>) -> Result<Vec<PathBuf>, CliError> {
    let mut out = Vec::new();
    if snapshots.is_empty() {
        return Ok(out);
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(io_error(dir, &e)),
    };
    for entry in entries {
        let path = entry.map_err(|e| io_error(dir, &e))?.path();
        if !path.is_file() || path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| io_error(&path, &e))?;
        let report: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let uses = report["provenance"]["snapshots"]
            .as_array()
            .is_some_and(|s| {
                s.iter()
                    .any(|s| s.as_str().is_some_and(|s| snapshots.contains(s)))
            });
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
    let mut sets = Vec::new();
    for x in data::in_force(&project)? {
        let records = snapshot::read(&store, &x.snapshot).map_err(CliError::store)?;
        sets.push((x.snapshot, records, data::provenance_of(&store, &x)?));
    }
    let items: BTreeSet<String> = sets
        .iter()
        .flat_map(|(_, rs, _)| content::items_naming(rs, &body_ref))
        .collect();
    let named: BTreeSet<&str> = sets
        .iter()
        .flat_map(|(_, rs, _)| rs.iter().flat_map(content::bodies))
        .filter(|r| *r != body_ref)
        .collect();
    let mut doomed = content::unshared_copies(
        sets.iter().flat_map(|(_, _, p)| &p.redactions),
        &named,
        &body,
    );
    doomed.extend(store.contains(&body).then_some(body));
    if items.is_empty() && doomed.is_empty() {
        return Err(CliError::usage(format!(
            "no record names {body_ref} and it is not stored"
        )));
    }
    let mut replaced = Vec::new();
    let mut records_changed = 0;
    let (mut snapshots, mut read) = (Vec::new(), Vec::new());
    for (old, mut records, prov) in sets {
        let n = content::purge_records(&mut records, &items, &body_ref).map_err(CliError::store)?;
        if n == 0 {
            snapshots.push(old);
            read.push(records);
            continue;
        }
        records_changed += n;
        let r = snapshot::replace(&store, old, &mut records, prov, |p| {
            p.redactions.retain(|r| r.original != body)
        })
        .map_err(CliError::store)?;
        snapshots.push(r.to);
        read.push(records);
        doomed.push(old);
        replaced.push(r);
    }
    let superseded: BTreeSet<String> = replaced.iter().map(|r| r.from.to_string()).collect();
    let reports = reports_using(&project.reports_dir(), &superseded)?;
    let report_names: Vec<String> = reports
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let summary = json!({
        "reason": args.reason,
        "replaced": replaced,
        "records_changed": records_changed,
        "deleted_objects": doomed,
        "deleted_reports": report_names,
    });
    let mut results = summary.clone();
    results["body"] = json!(body_ref);
    results["items"] = json!(items);
    let mut envelope = Envelope::new("evalsig.purge/1", results);
    // The ledger names the replacements before anything is deleted, so an
    // interrupted purge leaves text behind rather than data that cannot be loaded.
    project.record(
        &mut envelope,
        Kind::Purge,
        std::iter::once(body_ref).chain(superseded).collect(),
        Replacement::outputs(&replaced),
        summary,
    )?;
    for h in &doomed {
        store.remove(h).map_err(CliError::store)?;
    }
    for p in &reports {
        std::fs::remove_file(p).map_err(|e| io_error(p, &e))?;
    }
    if project.cache_path().exists() {
        project.rebuild_cache(&data::layered(snapshots, read))?;
    }
    Ok(Output::success(envelope))
}
