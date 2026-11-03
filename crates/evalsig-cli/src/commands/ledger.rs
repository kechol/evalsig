//! `evalsig ledger show|verify|anchor`: read, check and anchor a ledger.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use evalsig_store::ledger::{self, LedgerSide, Problem};
use serde_json::json;

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode};
use crate::project::{Project, io_error};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum SideArg {
    Agent,
    Referee,
}

#[derive(Debug, Subcommand)]
pub enum LedgerCommand {
    /// List the entries of a ledger.
    Show {
        /// The ledger file (default: the project's agent-side ledger).
        #[arg(long, value_name = "FILE")]
        ledger: Option<PathBuf>,
        /// Show only the last N entries.
        #[arg(long, value_name = "N")]
        last: Option<usize>,
    },
    /// Check the hash chain, and optionally the head against an anchor.
    Verify {
        #[arg(long, value_name = "FILE")]
        ledger: Option<PathBuf>,
        /// Which ledger this is; entries of the other ledger or its kinds are reported.
        #[arg(long, value_enum)]
        side: Option<SideArg>,
        /// A head hash recorded elsewhere earlier (git, CI log, pull request). A
        /// truncated or rebuilt chain does not contain it.
        #[arg(long, value_name = "HASH")]
        anchor: Option<String>,
    },
    /// Print the head hash to record outside the ledger.
    Anchor {
        #[arg(long, value_name = "FILE")]
        ledger: Option<PathBuf>,
    },
}

fn path_of(root: &Path, ledger: &Option<PathBuf>) -> Result<PathBuf, CliError> {
    match ledger {
        Some(p) => Ok(p.clone()),
        None => Ok(Project::open(root)?.ledger_path()),
    }
}

pub fn run(root: &Path, cmd: &LedgerCommand) -> Result<Output, CliError> {
    match cmd {
        LedgerCommand::Show { ledger, last } => {
            let path = path_of(root, ledger)?;
            let entries = ledger::read(&path).map_err(CliError::store)?;
            let skip = last.map_or(0, |n| entries.len().saturating_sub(n));
            let rows: Vec<_> = entries
                .iter()
                .skip(skip)
                .map(|e| {
                    json!({
                        "seq": e.seq, "ts": e.ts, "actor": e.actor, "kind": e.kind,
                        "ledger": e.ledger, "outputs": e.outputs, "hash": e.hash,
                    })
                })
                .collect();
            Ok(Output::success(Envelope::new(
                "evalsig.ledger.show/1",
                json!({ "ledger": path.display().to_string(), "count": entries.len(), "entries": rows }),
            )))
        }
        LedgerCommand::Verify {
            ledger,
            side,
            anchor,
        } => {
            let path = path_of(root, ledger)?;
            let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, &e))?;
            let side = side.map(|s| match s {
                SideArg::Agent => LedgerSide::Agent,
                SideArg::Referee => LedgerSide::Referee,
            });
            let mut problems: Vec<String> = ledger::verify(&text, side)
                .iter()
                .map(Problem::to_string)
                .collect();
            let entries = ledger::read(&path).map_err(CliError::store)?;
            if let Some(a) = anchor
                && !entries.iter().any(|e| &e.hash == a)
            {
                problems.push(format!(
                    "anchor {a} is not in the chain (truncated or rebuilt?)"
                ));
            }
            let head = ledger::head(&entries).to_owned();
            let mut envelope = Envelope::new(
                "evalsig.ledger.verify/1",
                json!({
                    "ledger": path.display().to_string(),
                    "entries": entries.len(),
                    "head": head,
                    "anchor_checked": anchor.is_some(),
                    "problems": problems,
                }),
            );
            if problems.is_empty() {
                return Ok(Output::success(envelope));
            }
            envelope.verdict = Some(Verdict {
                code: VerdictCode::Blocked,
                text: format!(
                    "The ledger fails verification: {} problems.",
                    problems.len()
                ),
            });
            Ok(Output {
                envelope,
                exit: Exit::Blocked,
            })
        }
        LedgerCommand::Anchor { ledger } => {
            let path = path_of(root, ledger)?;
            let entries = ledger::read(&path).map_err(CliError::store)?;
            Ok(Output::success(Envelope::new(
                "evalsig.ledger.anchor/1",
                json!({ "ledger": path.display().to_string(), "head": ledger::head(&entries), "seq": entries.last().map(|e| e.seq) }),
            )))
        }
    }
}
