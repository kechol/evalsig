//! `evalsig report <run>`: the default-tier metrics of one run.

use evalsig_project::report::Request;

use crate::args::{AnalysisArgs, DatasetArgs};

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
            checks: a.checks.into(),
        }
    }
}
