//! `evalsig seal`: encrypt a holdout's reference labels to the referee's recipient
//! (SPEC §8.2, audit §7, D-013).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use evalsig_core::records::Record;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use evalsig_store::{seal, snapshot};
use serde_json::{Value, json};

use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output};
use crate::project::{Project, hex, io_error};
use crate::referee::Referee;

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The holdout to seal, by the hash `split --holdout` reported.
    #[arg(long, value_name = "HASH")]
    pub holdout: String,
    /// The agent's project, which must hold no label of a holdout item.
    #[arg(long, value_name = "DIR")]
    pub agent_project: PathBuf,
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let (referee, _lock) = Referee::open_exclusive(args.referee.as_deref())?;
    let held = referee.holdout_items(&args.holdout)?;
    // The agent's environment must never have held these labels (SPEC §8.2). Snapshots
    // that `exclude` or a re-import replaced stay in the store, so every stored object
    // is searched line by line: a line this version cannot read does not hide the rest.
    let store = Project::open(&args.agent_project)?.objects();
    let mut leaked = BTreeSet::new();
    for h in store.list().map_err(CliError::store)? {
        let bytes = store.get(&h).map_err(CliError::store)?;
        for line in snapshot::lines(&bytes) {
            let Some(Record::Label(l)) = std::str::from_utf8(line)
                .ok()
                .and_then(|t| Record::from_json(t).ok())
            else {
                continue;
            };
            if held.contains(&l.item_id) {
                leaked.insert(l.item_id);
            }
        }
    }
    if !leaked.is_empty() {
        return Err(CliError::new(
            "holdout_labels_in_agent",
            format!(
                "the agent's snapshots hold labels of {} holdout items; cut a new holdout from items the agent has no labels for",
                leaked.len()
            ),
            Exit::Rejected,
        ));
    }
    let project = Project::open(root)?;
    let d = data::load(&project)?;
    // Every revision of every holdout label, canonical, one per line.
    let mut lines: Vec<Vec<u8>> = d
        .records
        .iter()
        .filter(|r| matches!(r, Record::Label(l) if held.contains(&l.item_id)))
        .map(|r| {
            let v = serde_json::to_value(r).expect("records serialize");
            to_canonical_bytes(&v).map_err(CliError::store)
        })
        .collect::<Result<_, _>>()?;
    lines.sort();
    lines.dedup();
    if lines.is_empty() {
        return Err(CliError::new(
            "insufficient_data",
            "the referee's project holds no label of the holdout items",
            Exit::InsufficientData,
        ));
    }
    let plaintext: Vec<u8> = lines
        .iter()
        .flat_map(|l| l.iter().copied().chain([b'\n']))
        .collect();
    let sealed = seal::seal(&referee.recipient()?, &plaintext)
        .map_err(|e| CliError::new("seal", e.to_string(), Exit::Rejected))?;
    let path = referee.file("sealed", &args.holdout, "age");
    std::fs::write(&path, &sealed).map_err(|e| io_error(&path, &e))?;
    let sealed_ref = ContentHash::blake3(&sealed).to_string();

    // Revisions after the first seal leave only hashes in the ledger (SPEC §8.2).
    let label_hashes: BTreeSet<String> = lines
        .iter()
        .map(|l| ContentHash::blake3(l).to_string())
        .collect();
    let state = referee
        .path("state")
        .join(format!("{}.labels.json", hex(&args.holdout)));
    let previous: Option<BTreeSet<String>> = std::fs::read_to_string(&state)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let snapshots = d.snapshot_ids();
    if let Some(prev) = &previous {
        let revised: Vec<&String> = label_hashes.difference(prev).collect();
        if !revised.is_empty() {
            referee.record(
                Kind::LabelRevision,
                snapshots.clone(),
                vec![sealed_ref.clone()],
                json!({ "holdout": args.holdout, "labels": revised }),
            )?;
        }
    }
    std::fs::write(
        &state,
        serde_json::to_string(&label_hashes).expect("hashes serialize"),
    )
    .map_err(|e| io_error(&state, &e))?;
    let summary = json!({
        "holdout": args.holdout,
        "sealed": sealed_ref,
        "labels": lines.len(),
        "items": held.len(),
        "resealed": previous.is_some(),
    });
    referee.record(
        Kind::Seal,
        snapshots.clone(),
        vec![sealed_ref],
        summary.clone(),
    )?;
    let mut results: Value = summary;
    results["file"] = json!(path.display().to_string());
    let mut envelope = Envelope::new("evalsig.seal/1", results);
    envelope.provenance.snapshots = snapshots;
    envelope.provenance.ledger = Some("referee".into());
    Ok(Output::success(envelope))
}
