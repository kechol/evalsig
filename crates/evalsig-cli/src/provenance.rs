//! Provenance shared by result-producing commands: the snapshots, the stored
//! configuration, the seed, the request and the output hash; the saved report file;
//! and the ledger line that names it.

use evalsig_core::config::Config;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use serde_json::Value;

use crate::data::Data;
use crate::error::CliError;
use crate::output::Envelope;
use crate::project::{Project, io_error};

/// The hash of canonical results.
pub fn output_hash(results: &Value) -> Result<ContentHash, CliError> {
    Ok(ContentHash::blake3(
        &to_canonical_bytes(results).map_err(CliError::store)?,
    ))
}

/// Fill the provenance, save the envelope under `reports/` by its output hash and,
/// when `kind` is given, record it in the agent-side ledger.
#[allow(clippy::too_many_arguments)]
pub fn finish(
    project: &Project,
    config: &Config,
    d: &Data,
    envelope: &mut Envelope,
    seed: u64,
    request: Value,
    kind: Kind,
    summary: Value,
) -> Result<(), CliError> {
    let out = output_hash(&envelope.results)?;
    let cfg = project.store_config(config)?;
    envelope.provenance.snapshots = d.snapshots.iter().map(ToString::to_string).collect();
    envelope.provenance.config = Some(cfg.to_string());
    envelope.provenance.seed = Some(seed);
    envelope.provenance.request = Some(request);
    envelope.provenance.output = Some(out.to_string());
    // Save before the ledger names the output, so a recorded output always exists.
    save(project, envelope, &out)?;
    let inputs = envelope
        .provenance
        .snapshots
        .iter()
        .cloned()
        .chain([cfg.to_string()])
        .collect();
    project.record(
        envelope,
        kind,
        inputs,
        vec![out.to_string()],
        summary.as_object().cloned().unwrap_or_default(),
    )
}

pub fn save(project: &Project, envelope: &Envelope, out: &ContentHash) -> Result<(), CliError> {
    let path = project
        .dir
        .join("reports")
        .join(format!("{}.json", out.hex()));
    let mut text = serde_json::to_string_pretty(envelope).expect("envelope serializes");
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| io_error(&path, &e))
}
