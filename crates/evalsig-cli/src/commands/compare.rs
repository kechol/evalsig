//! `evalsig compare <baseline> <candidate>...`: a paired comparison on common items.

use evalsig_core::split::Part;
use evalsig_project::compare::Request;

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
    /// Give each feature lineage's likelihood ratio in both runs, a lineage firing when
    /// any of its features does (D-049).
    #[arg(long)]
    pub features: bool,
    /// Add ΔH, ΔF, Δd′ and Δc with paired intervals, and whether discrimination or the
    /// criterion changed (D-042).
    #[arg(long, conflicts_with = "unpaired")]
    pub sdt: bool,
    /// With --sdt: the fewest weighted items each cell (hits, misses, false alarms,
    /// correct rejections) of each arm needs for a classification.
    #[arg(long, value_name = "N", default_value_t = 5.0, requires = "sdt", value_parser = at_least_zero)]
    pub min_events: f64,
    #[command(flatten)]
    pub dataset: DatasetArgs,
    #[command(flatten)]
    pub arms: ArmArgs,
}

fn at_least_zero(s: &str) -> Result<f64, String> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() && v >= 0.0 => Ok(v),
        _ => Err("a finite number of at least 0".into()),
    }
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
                SplitArg::Dev => Part::Dev,
                SplitArg::Validation => Part::Validation,
            }),
            features: a.features,
            sdt: a.sdt.then_some(a.min_events),
            dataset: a.dataset.into(),
            arms: a.arms.into(),
        }
    }
}
