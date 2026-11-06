//! `evalsig ledger show|verify|anchor`: read, check and anchor a ledger.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use evalsig_store::ledger::{self, LedgerSide, Problem};
use serde_json::json;

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output, Schema, Verdict, VerdictCode};
use evalsig_project::exit::Exit;
use evalsig_project::project::{Project, io_error};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum SideArg {
    Agent,
    Referee,
}

impl From<SideArg> for LedgerSide {
    fn from(s: SideArg) -> Self {
        match s {
            SideArg::Agent => LedgerSide::Agent,
            SideArg::Referee => LedgerSide::Referee,
        }
    }
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

/// The ledger to read: a named file, which must exist, or the project's own, which
/// is empty until the first entry.
fn path_of(root: &Path, ledger: Option<&Path>) -> Result<PathBuf, Error> {
    match ledger {
        Some(p) if !p.is_file() => Err(Error::usage(format!("no ledger file at {}", p.display()))),
        Some(p) => Ok(p.to_path_buf()),
        None => Ok(Project::open(root)?.ledger_path()),
    }
}

pub fn run(root: &Path, cmd: &LedgerCommand) -> Result<Output, Error> {
    match cmd {
        LedgerCommand::Show { ledger, last } => {
            let path = path_of(root, ledger.as_deref())?;
            let entries = ledger::read(&path).map_err(Error::store)?;
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
                Schema::LedgerShow,
                json!({ "ledger": path.display().to_string(), "count": entries.len(), "entries": rows }),
            )))
        }
        LedgerCommand::Verify {
            ledger,
            side,
            anchor,
        } => {
            let path = path_of(root, ledger.as_deref())?;
            let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, &e))?;
            let (entries, found) = ledger::check(&text, side.map(LedgerSide::from));
            let mut problems: Vec<String> = found.iter().map(Problem::to_string).collect();
            if let Some(a) = anchor
                && !entries.iter().any(|e| &e.hash == a)
            {
                problems.push(format!(
                    "anchor {a} is not in the chain (truncated or rebuilt?)"
                ));
            }
            let head = ledger::head(&entries).to_owned();
            let mut envelope = Envelope::new(
                Schema::LedgerVerify,
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
            let path = path_of(root, ledger.as_deref())?;
            let entries = ledger::read(&path).map_err(Error::store)?;
            Ok(Output::success(Envelope::new(
                Schema::LedgerAnchor,
                json!({ "ledger": path.display().to_string(), "head": ledger::head(&entries), "seq": entries.last().map(|e| e.seq) }),
            )))
        }
    }
}
