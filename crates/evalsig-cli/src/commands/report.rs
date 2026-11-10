//! `evalsig report <run>`: the default-tier metrics of one run.

use evalsig_core::attribution::Errors;
use evalsig_project::report::Request;

use crate::args::{AnalysisArgs, DatasetArgs};

/// Which errors `--decompose` splits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum ErrorsArg {
    Misses,
    FalseAlarms,
}

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The run to report.
    pub run: String,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
    /// Also report each value of this stratum. `reference_agreement` splits the items
    /// two or more human raters labelled into `unanimous` and `split`.
    #[arg(long, value_name = "STRATUM")]
    pub by: Option<String>,
    /// Include one row per item.
    #[arg(long)]
    pub items: bool,
    /// Also report majority@k, any@k and all@k for k = 1 … the fewest replicates of
    /// any item, with the expected cost per item.
    #[arg(long)]
    pub k_curve: bool,
    /// Add each feature's likelihood ratio and class.
    #[arg(long)]
    pub features: bool,
    /// With --reference human: correct the human-scale value with this model rater's
    /// labels on the items humans did not label (PPI++).
    #[arg(long, value_name = "RATER")]
    pub proxy: Option<String>,
    /// Split the misses or false alarms by the causes attributions name, each error to
    /// its first cause in `[attribution] order` (D-048).
    #[arg(long, value_enum)]
    pub decompose: Option<ErrorsArg>,
    /// With --decompose: the rate left if the errors of these causes (comma-separated)
    /// were removed; an optimistic bound. Repeat for several sets.
    #[arg(long, value_name = "CAUSES", requires = "decompose")]
    pub ceiling: Vec<String>,
    #[command(flatten)]
    pub checks: DatasetArgs,
}

impl From<&Args> for Request {
    fn from(a: &Args) -> Self {
        Request {
            run: a.run.clone(),
            analysis: a.analysis.clone().into(),
            by: a.by.clone(),
            items: a.items,
            k_curve: a.k_curve,
            features: a.features,
            proxy: a.proxy.clone(),
            decompose: a.decompose.map(|e| match e {
                ErrorsArg::Misses => Errors::Misses,
                ErrorsArg::FalseAlarms => Errors::FalseAlarms,
            }),
            ceiling: a
                .ceiling
                .iter()
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|c| !c.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .collect(),
            checks: a.checks.into(),
        }
    }
}
