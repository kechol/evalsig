//! `evalsig trend <run>`: a run's metrics per period, compared with a baseline period.

use evalsig_project::trend::{By, Request};

use crate::args::AnalysisArgs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ByArg {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The run to follow over time.
    pub run: String,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
    /// The period length.
    #[arg(long, value_enum, default_value_t = ByArg::Week)]
    pub by: ByArg,
    /// Only items that occurred at or after this RFC 3339 time.
    #[arg(long, value_name = "TIME")]
    pub since: Option<String>,
    /// The period the others are compared with (default: the first).
    #[arg(long, value_name = "PERIOD")]
    pub baseline_period: Option<String>,
}

impl From<&Args> for Request {
    fn from(a: &Args) -> Self {
        Request {
            run: a.run.clone(),
            analysis: a.analysis.clone().into(),
            by: match a.by {
                ByArg::Day => By::Day,
                ByArg::Week => By::Week,
                ByArg::Month => By::Month,
            },
            since: a.since.clone(),
            baseline_period: a.baseline_period.clone(),
        }
    }
}
