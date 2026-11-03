//! Provenance shared by result-producing commands: the snapshots, the stored
//! configuration, the seed, the request and the output hash; the saved report file;
//! and the ledger line that names it.

use evalsig_core::config::Config;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use serde_json::Value;

use crate::commands::{compare, report};
use crate::data::Data;
use crate::error::CliError;
use crate::output::Envelope;
use crate::project::{Project, io_error};

/// The command and arguments a result was computed from, which `verify` repeats.
/// Serialized as `{"command": ..., "args": ...}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub enum Request {
    Report(report::Args),
    Compare(compare::Args),
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
pub fn output_hash(results: &Value) -> Result<ContentHash, CliError> {
    Ok(ContentHash::blake3(
        &to_canonical_bytes(results).map_err(CliError::store)?,
    ))
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
) -> Result<ContentHash, CliError> {
    let out = output_hash(&envelope.results)?;
    let p = &mut envelope.provenance;
    p.snapshots = d.snapshots.iter().map(ToString::to_string).collect();
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
) -> Result<(), CliError> {
    let out = fill(project, config, d, envelope, seed, request)?;
    // Save before the ledger names the output, so a recorded output always exists.
    let path = project.reports_dir().join(format!("{}.json", out.hex()));
    let mut text = serde_json::to_string_pretty(envelope).expect("envelope serializes");
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| io_error(&path, &e))?;
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
