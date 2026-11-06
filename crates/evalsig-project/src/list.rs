//! `evalsig runs` and `evalsig items`: what has been imported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use evalsig_core::records::Record;
use serde_json::{Value, json};

use crate::Error;
use crate::data;
use crate::envelope::{Envelope, Output};
use crate::project::Project;

fn envelope(
    schema: &str,
    results: Value,
    snapshots: &[evalsig_store::hash::ContentHash],
) -> Output {
    let mut e = Envelope::new(schema, results);
    e.provenance.snapshots = snapshots.iter().map(ToString::to_string).collect();
    Output::success(e)
}

pub fn runs(root: &Path) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let d = data::load(&project)?;
    let mut judged: BTreeMap<&str, (u64, BTreeSet<&str>, u64)> = BTreeMap::new();
    for r in &d.records {
        if let Record::Judgment(j) = r {
            let e = judged.entry(j.run_id.as_str()).or_default();
            e.0 += 1;
            e.1.insert(j.item_id.as_str());
            e.2 += u64::from(j.missing.is_some());
        }
    }
    let runs: Vec<Value> = d
        .records
        .iter()
        .filter_map(|r| match r {
            Record::Run(run) => {
                let (n, items, missing) =
                    judged.get(run.run_id.as_str()).cloned().unwrap_or_default();
                Some(json!({
                    "run_id": run.run_id,
                    "system": run.system.id,
                    "version": run.system.version,
                    "value_semantics": run.value_semantics(),
                    "output_type": run.output_type,
                    "levels": run.levels,
                    "grader": run.grader.as_ref().and_then(|g| g.id.clone()),
                    "items": items.len(),
                    "judgments": n,
                    "missing": missing,
                    "started_at": run.started_at,
                }))
            }
            _ => None,
        })
        .collect();
    Ok(envelope(
        "evalsig.runs/1",
        json!({ "count": runs.len(), "runs": runs }),
        &d.snapshots,
    ))
}

pub fn items(root: &Path) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let d = data::load(&project)?;
    let mut labels: BTreeMap<&str, u64> = BTreeMap::new();
    let mut judgments: BTreeMap<&str, u64> = BTreeMap::new();
    for r in &d.records {
        match r {
            Record::Label(l) => *labels.entry(l.item_id.as_str()).or_default() += 1,
            Record::Judgment(j) => *judgments.entry(j.item_id.as_str()).or_default() += 1,
            _ => {}
        }
    }
    let items: Vec<Value> = d
        .records
        .iter()
        .filter_map(|r| match r {
            Record::Item(i) => Some(json!({
                "item_id": i.item_id,
                "cluster": i.cluster_key(),
                "strata": i.strata.clone().unwrap_or_default(),
                "weight": i.weight(),
                "labels": labels.get(i.item_id.as_str()).copied().unwrap_or(0),
                "judgments": judgments.get(i.item_id.as_str()).copied().unwrap_or(0),
                "occurred_at": i.occurred_at,
            })),
            _ => None,
        })
        .collect();
    Ok(envelope(
        "evalsig.items/1",
        json!({ "count": items.len(), "items": items }),
        &d.snapshots,
    ))
}

/// `evalsig capabilities [--run]`: the analyses the imported data allow, and what to
/// add for the others.
pub fn capabilities(root: &Path, run: Option<&str>) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let d = data::load(&project)?;
    if let Some(r) = run {
        d.run(r)?;
    }
    let c = evalsig_core::capabilities::of(&d.records, run);
    let mut envelope = Envelope::new(
        "evalsig.capabilities/1",
        serde_json::json!({"run": run, "capabilities": c}),
    );
    envelope.provenance.snapshots = d.snapshot_ids();
    Ok(Output::success(envelope))
}
