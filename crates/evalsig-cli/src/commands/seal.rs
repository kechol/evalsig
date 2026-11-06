//! `evalsig seal`: encrypt a holdout's reference labels to the referee's recipient
//! (SPEC §8.2, audit §7, D-013).

use std::path::{Path, PathBuf};

use evalsig_project::Error;
use evalsig_project::envelope::Output;
use evalsig_referee::seal::{self, Request};

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

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    seal::run(
        root,
        &Request {
            holdout: args.holdout.clone(),
            agent_project: args.agent_project.clone(),
            referee: args.referee.clone(),
        },
    )
}
