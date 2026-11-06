//! Provenance shared by result-producing commands: the snapshots, the stored
//! configuration, the seed, the request and the output hash; the saved report file;
//! and the ledger line that names it.

use evalsig_core::config::Config;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use serde_json::Value;

use crate::Error;
use crate::data::Data;
use crate::envelope::Envelope;
use crate::project::{Project, io_error};
use crate::{compare, report};

/// The command and arguments a result was computed from, which `verify` repeats.
/// Serialized as `{"command": ..., "args": ...}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum Request {
    Report(report::Request),
    Compare(compare::Request),
}

impl Request {
    fn kind(&self) -> Kind {
        match self {
            Request::Report(_) => Kind::Report,
            Request::Compare(_) => Kind::Compare,
        }
    }
}

/// The hash of canonical results.
pub fn output_hash(results: &Value) -> Result<ContentHash, Error> {
    evalsig_store::canonical::hash(results).map_err(Error::store)
}

/// The name of an item set (a split, the items a comparison evaluates): the BLAKE3
/// reference of the canonical JSON array of its sorted item IDs (D-022).
pub fn item_set_hash<'a>(items: impl Iterator<Item = &'a String>) -> Result<String, Error> {
    Ok(ContentHash::blake3(&item_set_bytes(items)?).to_string())
}

/// The canonical JSON array of the sorted item IDs that `item_set_hash` names.
pub fn item_set_bytes<'a>(items: impl Iterator<Item = &'a String>) -> Result<Vec<u8>, Error> {
    let mut ids: Vec<&String> = items.collect();
    ids.sort();
    to_canonical_bytes(&serde_json::json!(ids)).map_err(Error::store)
}

/// Fill the provenance with what the result was computed from; returns the output
/// hash.
pub fn fill(
    project: &Project,
    config: &Config,
    d: &Data,
    envelope: &mut Envelope,
    seed: u64,
    request: &Request,
) -> Result<ContentHash, Error> {
    let out = output_hash(&envelope.results)?;
    let p = &mut envelope.provenance;
    p.snapshots = d.snapshot_ids();
    p.config = Some(project.store_config(config)?.to_string());
    p.seed = Some(seed);
    p.request = Some(serde_json::to_value(request).expect("requests serialize"));
    p.output = Some(out.to_string());
    Ok(out)
}

/// Fill the provenance, save the envelope under `reports/` by its output hash and
/// record it in the agent-side ledger.
pub fn finish(
    project: &Project,
    config: &Config,
    d: &Data,
    envelope: &mut Envelope,
    seed: u64,
    request: &Request,
    summary: Value,
) -> Result<(), Error> {
    let out = fill(project, config, d, envelope, seed, request)?;
    // Save before the ledger names the output, so a recorded output always exists.
    let path = project.reports_dir().join(format!("{}.json", out.hex()));
    std::fs::write(&path, envelope.to_pretty_json()).map_err(|e| io_error(&path, &e))?;
    let p = &envelope.provenance;
    let inputs = p.snapshots.iter().chain(&p.config).cloned().collect();
    project.record(
        envelope,
        request.kind(),
        inputs,
        vec![out.to_string()],
        summary,
    )
}
