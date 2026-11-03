//! `evalsig preregister --draft gate.toml`: check the conditions against the gate
//! schema, name them by the hash of their canonical form and record the draft in the
//! agent-side ledger (SPEC §8.3).

use std::path::{Path, PathBuf};

use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use serde_json::{Value, json};

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output};
use crate::project::{Project, io_error};
use crate::schemas;

#[derive(Debug, Clone, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    /// Draft these conditions (gate.toml).
    #[arg(long, value_name = "FILE")]
    pub draft: Option<PathBuf>,
    #[command(subcommand)]
    pub action: Option<Action>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// As the referee: resolve and pin a draft into approved conditions.
    Approve(crate::commands::approve::ApproveArgs),
    /// As the referee: print approved conditions as gate reads them.
    Show {
        /// The approved conditions' hash.
        approved: String,
        /// The referee's home (default: EVALSIG_REFEREE).
        #[arg(long, value_name = "DIR")]
        referee: Option<PathBuf>,
    },
}

fn invalid(message: impl Into<String>) -> CliError {
    CliError::new("invalid_conditions", message, Exit::Rejected)
}

/// The conditions as JSON, checked against `gate/1`; refuses the `resolved` section,
/// which only approval writes.
pub fn parse(path: &Path) -> Result<Value, CliError> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, &e))?;
    let toml: toml::Table = toml::from_str(&text)
        .map_err(|e: toml::de::Error| invalid(format!("{}: {e}", path.display())))?;
    let doc = serde_json::to_value(toml).expect("TOML converts to JSON");
    if doc.get("resolved").is_some() {
        return Err(invalid(
            "a draft has no resolved section; approval writes it",
        ));
    }
    let schema: Value = serde_json::from_str(schemas::GATE).expect("embedded schema is JSON");
    let validator = jsonschema::options()
        .build(&schema)
        .expect("embedded schema compiles");
    let errors: Vec<String> = validator
        .iter_errors(&doc)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    if !errors.is_empty() {
        return Err(invalid(format!(
            "{}: {}",
            path.display(),
            errors.join("; ")
        )));
    }
    Ok(doc)
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    match (&args.draft, &args.action) {
        (_, Some(Action::Approve(a))) => crate::commands::approve::approve(root, a),
        (_, Some(Action::Show { approved, referee })) => {
            let r = crate::referee::Referee::open(referee.as_deref())?;
            let doc = crate::commands::approve::load(&r, approved)?;
            Ok(Output::success(Envelope::new(
                "evalsig.preregister/1",
                json!({ "approved": approved, "conditions": doc }),
            )))
        }
        (Some(file), None) => draft(root, file),
        (None, None) => Err(CliError::usage("give --draft <gate.toml>, approve or show")),
    }
}

fn draft(root: &Path, file: &Path) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let doc = parse(file)?;
    let canonical = to_canonical_bytes(&doc).map_err(CliError::store)?;
    let hash = ContentHash::blake3(&canonical);
    let dir = project.dir.join("drafts");
    std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, &e))?;
    let path = dir.join(format!("{}.json", hash.hex()));
    std::fs::write(&path, &canonical).map_err(|e| io_error(&path, &e))?;
    let summary = json!({
        "name": doc["name"],
        "draft": hash.to_string(),
        "split": doc["split"],
        "conditions": doc["conditions"].as_array().map_or(0, Vec::len),
    });
    let mut envelope = Envelope::new(
        "evalsig.preregister/1",
        json!({
            "draft": hash.to_string(),
            "name": doc["name"],
            "conditions": doc["conditions"],
            "file": format!(".evalsig/drafts/{}.json", hash.hex()),
            "approved": false,
        }),
    );
    project.record(
        &mut envelope,
        Kind::PreregisterDraft,
        vec![],
        vec![hash.to_string()],
        summary,
    )?;
    Ok(Output::success(envelope))
}
