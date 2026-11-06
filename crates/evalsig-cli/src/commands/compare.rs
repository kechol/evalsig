//! `evalsig compare <baseline> <candidate>...`: a paired comparison on common items.

use evalsig_project::compare::Request;
use evalsig_project::split::SplitPart;

use crate::args::{AnalysisArgs, ArmArgs, DatasetArgs};

/// A part of an agent-side split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SplitArg {
    Dev,
    Validation,
}

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The baseline run.
    pub baseline: String,
    /// One or more candidate runs.
    #[arg(required = true)]
    pub candidates: Vec<String>,
    /// The metric that decides the verdict (default: the first the runs support).
    #[arg(long)]
    pub metric: Option<String>,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
    /// Include one row per paired item (`--format jsonl` prints only these rows).
    #[arg(long)]
    pub items: bool,
    /// Do not record the comparison in the ledger.
    #[arg(long)]
    pub scratch: bool,
    /// Compare the arms as independent samples whose items need not overlap
    /// (Newcombe, Welch or a bootstrap per arm).
    #[arg(long, conflicts_with = "sequential")]
    pub unpaired: bool,
    /// Also add the paired items one by one in a seeded order and report the
    /// confidence sequence, which stays valid however often it is inspected.
    #[arg(long)]
    pub sequential: bool,
    /// Compare only the items of this part of the latest `evalsig split`.
    #[arg(long, value_enum)]
    pub split: Option<SplitArg>,
    #[command(flatten)]
    pub dataset: DatasetArgs,
    #[command(flatten)]
    pub arms: ArmArgs,
}

impl From<&Args> for Request {
    fn from(a: &Args) -> Self {
        Request {
            baseline: a.baseline.clone(),
            candidates: a.candidates.clone(),
            metric: a.metric.clone(),
            analysis: a.analysis.clone().into(),
            items: a.items,
            scratch: a.scratch,
            unpaired: a.unpaired,
            sequential: a.sequential,
            split: a.split.map(|s| match s {
                SplitArg::Dev => SplitPart::Dev,
                SplitArg::Validation => SplitPart::Validation,
            }),
            dataset: a.dataset.into(),
            arms: a.arms.into(),
        }
    }
}
