//! `evalsig dashboard`: serve the local web dashboard (SPEC §11, D-019).

use std::path::Path;

use evalsig_project::Error;
use evalsig_project::envelope::Output;

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The port on 127.0.0.1 (0 picks a free one).
    #[arg(long, default_value_t = 7777)]
    pub port: u16,
}

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    evalsig_dashboard::screens::run(root, args.port)
}
