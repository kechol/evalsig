//! Argument groups several commands flatten in, and their conversion into the
//! project's request types.

use evalsig_core::config::Estimand;
use evalsig_core::records::DEFAULT_TASK;
use evalsig_project::analysis::{Analysis, ReferenceChoice};
use evalsig_project::checks::{ArmChecks, DatasetChecks};

#[derive(Debug, Clone, clap::Args)]
pub struct AnalysisArgs {
    /// The judgment task.
    #[arg(long, default_value = DEFAULT_TASK)]
    pub task: String,
    /// The estimand: single_shot, majority@k, any@k or all@k (default from the configuration).
    #[arg(long)]
    pub estimand: Option<Estimand>,
    /// Decision threshold for score outputs; for score runs, also the share at or above it.
    #[arg(long)]
    pub threshold: Option<f64>,
    /// Use the labels as they were at this RFC 3339 time.
    #[arg(long, value_name = "TIME")]
    pub labels_as_of: Option<String>,
    /// How several raters' labels become the reference (default from the configuration).
    #[arg(long, value_enum)]
    pub reference: Option<ReferenceArg>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ReferenceArg {
    Single,
    Majority,
    Soft,
    Latent,
    /// The human labels, corrected with a model rater's labels (`report --proxy`).
    Human,
}

impl From<AnalysisArgs> for Analysis {
    fn from(a: AnalysisArgs) -> Self {
        Analysis {
            task: a.task,
            estimand: a.estimand,
            threshold: a.threshold,
            labels_as_of: a.labels_as_of,
            reference: a.reference.map(|r| match r {
                ReferenceArg::Single => ReferenceChoice::Single,
                ReferenceArg::Majority => ReferenceChoice::Majority,
                ReferenceArg::Soft => ReferenceChoice::Soft,
                ReferenceArg::Latent => ReferenceChoice::Latent,
                ReferenceArg::Human => ReferenceChoice::Human,
            }),
        }
    }
}

/// Flags for the checks that run over one dataset.
#[derive(Debug, Clone, Copy, Default, clap::Args)]
pub struct DatasetArgs {
    /// Continue although some missing values have no explained reason.
    #[arg(long)]
    pub allow_unexplained_missing: bool,
    /// Stop when content looks like a secret instead of warning.
    #[arg(long)]
    pub fail_on_secret: bool,
}

impl From<DatasetArgs> for DatasetChecks {
    fn from(a: DatasetArgs) -> Self {
        DatasetChecks {
            allow_unexplained_missing: a.allow_unexplained_missing,
            fail_on_secret: a.fail_on_secret,
        }
    }
}

/// Flags for the checks that compare two arms.
#[derive(Debug, Clone, Copy, Default, clap::Args)]
pub struct ArmArgs {
    /// Continue although the compared arms used different grader versions.
    #[arg(long)]
    pub allow_mixed_grader: bool,
    /// Keep items whose input differs between the compared arms.
    #[arg(long)]
    pub allow_input_change: bool,
}

impl From<ArmArgs> for ArmChecks {
    fn from(a: ArmArgs) -> Self {
        ArmChecks {
            allow_mixed_grader: a.allow_mixed_grader,
            allow_input_change: a.allow_input_change,
        }
    }
}
