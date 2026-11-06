//! `evalsig agree`: agreement between raters, of graders and runs with people, and on
//! preferences.

use evalsig_project::agree::Request;

use crate::args::AnalysisArgs;

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub analysis: AnalysisArgs,
    /// Also compare this run's decisions with each human rater, under the same rule
    /// as the raters among themselves.
    #[arg(long)]
    pub run: Option<String>,
    /// Agreement of raters on pairwise preferences, their position bias, and the
    /// Bradley-Terry ranking of the arms.
    #[arg(long)]
    pub preference: bool,
    /// The alternative annotator test: could the --run system replace one of the human
    /// raters (D-017)?
    #[arg(long, requires = "run")]
    pub parity: bool,
}

impl From<&Args> for Request {
    fn from(a: &Args) -> Self {
        Request {
            analysis: a.analysis.clone().into(),
            run: a.run.clone(),
            preference: a.preference,
            parity: a.parity,
        }
    }
}
