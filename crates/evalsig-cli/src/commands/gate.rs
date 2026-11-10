//! `evalsig gate`: judge a candidate on the sealed holdout under approved conditions,
//! serve submissions as the referee, or submit to a serving referee (SPEC §8.3, §8.5).

use std::path::{Path, PathBuf};

use evalsig_project::Error;
use evalsig_project::envelope::Output;
use evalsig_referee::gate::{Request, judge_request};
use evalsig_referee::home::Referee;
use evalsig_referee::serve::{self, Destination};

#[derive(Debug, Clone, clap::Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct Args {
    #[command(subcommand)]
    pub action: Option<Action>,
    #[command(flatten)]
    pub submission: Submission,
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// As the referee: answer submissions on a Unix socket in the referee home and,
    /// with --http, on token HTTP at 127.0.0.1.
    Serve(ServeArgs),
    /// Submit a candidate to a running `gate serve` and print its answer.
    Submit(SubmitArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct Submission {
    /// The approved conditions, by hash.
    #[arg(long, value_name = "HASH", required = true)]
    pub approved: Option<String>,
    /// The candidate's run ID.
    #[arg(long, value_name = "RUN", required = true)]
    pub candidate: Option<String>,
    /// The candidate's records (records/1 JSONL): its run and judgments.
    #[arg(long, value_name = "FILE", required = true)]
    pub records: Option<PathBuf>,
    /// The hash of the mapping or converter that made the records, as the candidate's
    /// side reports it.
    #[arg(long, value_name = "HASH")]
    pub mapping_hash: Option<String>,
    /// The agent-side ledger, whose head is copied into the gate line.
    #[arg(long, value_name = "FILE")]
    pub agent_ledger: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Args)]
pub struct ServeArgs {
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
    /// Also answer token HTTP on 127.0.0.1 at this port (0 picks a free one).
    #[arg(long, value_name = "PORT")]
    pub http: Option<u16>,
}

#[derive(Debug, Clone, clap::Args)]
pub struct SubmitArgs {
    #[command(flatten)]
    pub submission: Submission,
    /// The referee's Unix socket (`gate.sock` in its home).
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "url",
        required_unless_present = "url"
    )]
    pub socket: Option<PathBuf>,
    /// The referee's HTTP address, `http://127.0.0.1:<port>`.
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// A file holding the bearer token for --url (default: EVALSIG_GATE_TOKEN).
    #[arg(long, value_name = "FILE", requires = "url")]
    pub token_file: Option<PathBuf>,
}

impl Submission {
    /// The submission as the referee reads it: the records file's text and the head of
    /// the agent-side ledger, if one was named; a named ledger that cannot be read is
    /// an error, not a missing head.
    fn request(&self) -> Result<Request, Error> {
        let records = self.records.as_deref().expect("required by clap");
        let agent_ledger_head = match self.agent_ledger.as_ref() {
            Some(p) => evalsig_store::ledger::read(p)
                .map_err(|e| Error::usage(format!("--agent-ledger {}: {e}", p.display())))?
                .last()
                .map(|e| e.hash.clone()),
            None => None,
        };
        Ok(Request {
            approved: self.approved.clone().expect("required by clap"),
            candidate: self.candidate.clone().expect("required by clap"),
            records: read_records(records)?,
            mapping_hash: self.mapping_hash.clone(),
            agent_ledger_head,
        })
    }
}

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    match &args.action {
        Some(Action::Serve(a)) => serve::serve(root, a.referee.as_deref(), a.http),
        Some(Action::Submit(a)) => {
            let to = match (&a.socket, &a.url) {
                (Some(p), _) => Destination::Socket(p.clone()),
                (None, Some(url)) => Destination::Http {
                    url: url.clone(),
                    token_file: a.token_file.clone(),
                },
                (None, None) => unreachable!("clap requires --socket or --url"),
            };
            serve::submit(&a.submission.request()?, &to)
        }
        None => {
            let referee = Referee::open(args.referee.as_deref())?;
            judge_request(root, &referee, &args.submission.request()?)
        }
    }
}

/// The submitted records, decompressed when gzip or Zstandard (D-045).
fn read_records(path: &std::path::Path) -> Result<String, Error> {
    let bytes = crate::commands::import::read_within(path, &Default::default())?;
    String::from_utf8(bytes)
        .map_err(|_| Error::usage(format!("{} is not UTF-8 text", path.display())))
}
