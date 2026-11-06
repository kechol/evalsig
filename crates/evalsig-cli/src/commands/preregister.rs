//! `evalsig preregister --draft gate.toml`: check the conditions against the gate
//! schema, name them by the hash of their canonical form and record the draft in the
//! agent-side ledger (SPEC §8.3).

use std::path::{Path, PathBuf};

use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use serde_json::json;

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output};
use evalsig_project::project::{Project, io_error};
use evalsig_referee::approve;
use evalsig_referee::conditions::parse;
use evalsig_referee::home::Referee;

#[derive(Debug, Clone, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    /// Draft these conditions (gate.toml).
    #[arg(long, value_name = "FILE")]
    pub draft: Option<PathBuf>,
    /// Confirm the draft; agent mode requires it (D-018).
    #[arg(long)]
    pub yes: bool,
    #[command(subcommand)]
    pub action: Option<Action>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// As the referee: resolve and pin a draft into approved conditions.
    Approve(ApproveArgs),
    /// As the referee: print approved conditions as gate reads them.
    Show {
        /// The approved conditions' hash.
        approved: String,
        /// The referee's home (default: EVALSIG_REFEREE).
        #[arg(long, value_name = "DIR")]
        referee: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, clap::Args)]
pub struct ApproveArgs {
    /// The draft's hash, as `preregister --draft` reported it.
    pub draft: String,
    /// The holdout the gate will use, by its hash.
    #[arg(long, value_name = "HASH")]
    pub holdout: String,
    /// The project whose .evalsig/drafts/ holds the draft (default: --project).
    #[arg(long, value_name = "DIR")]
    pub from: Option<PathBuf>,
    /// The mapping file or converter that makes the candidate's records; its hash is
    /// pinned.
    #[arg(long, value_name = "FILE")]
    pub mapping: Option<PathBuf>,
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
}

pub fn run(root: &Path, args: &Args, agent: bool) -> Result<Output, Error> {
    match (&args.draft, &args.action) {
        (_, Some(Action::Approve(a))) => approve::approve(
            root,
            &approve::Request {
                draft: a.draft.clone(),
                holdout: a.holdout.clone(),
                from: a.from.clone(),
                mapping: a.mapping.clone(),
                referee: a.referee.clone(),
            },
        ),
        (_, Some(Action::Show { approved, referee })) => {
            let r = Referee::open(referee.as_deref())?;
            let doc = approve::load(&r, approved)?;
            Ok(Output::success(Envelope::new(
                "evalsig.preregister/1",
                json!({ "approved": approved, "conditions": doc }),
            )))
        }
        (Some(_), None) if agent && !args.yes => Err(Error::usage(
            "in agent mode a draft changes the procedure and needs confirming",
        )
        .with_hint("pass --yes; a person still approves the draft as the referee")),
        (Some(file), None) => draft(root, file),
        (None, None) => Err(Error::usage("give --draft <gate.toml>, approve or show")),
    }
}

fn draft(root: &Path, file: &Path) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let doc = parse(file)?;
    let canonical = to_canonical_bytes(&doc).map_err(Error::store)?;
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
