//! Options shared by the analysis commands (`report`, `compare`) and the settings
//! they resolve to.

use evalsig_core::config::{Config, Estimand, ReferenceMode};
use evalsig_core::records::DEFAULT_TASK;
use evalsig_core::report::Settings;

#[derive(Debug, Clone, clap::Args, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceArg {
    Single,
    Majority,
    Soft,
    Latent,
}

impl From<ReferenceArg> for ReferenceMode {
    fn from(r: ReferenceArg) -> Self {
        match r {
            ReferenceArg::Single => ReferenceMode::Single,
            ReferenceArg::Majority => ReferenceMode::Majority,
            ReferenceArg::Soft => ReferenceMode::Soft,
            ReferenceArg::Latent => ReferenceMode::Latent,
        }
    }
}

/// The settings for these options under a configuration; `prevalence`, `by` and
/// `items` are left for the command to set.
pub fn settings(config: &Config, a: &AnalysisArgs) -> Settings {
    Settings {
        task: a.task.clone(),
        positive: config.positive().for_task(&a.task).copied(),
        decision: config.decision().for_task(&a.task).copied(),
        threshold: a.threshold,
        estimand: a.estimand.unwrap_or(config.stats.estimand),
        missing: config.missing_policy(),
        reference: a.reference.map_or(config.reference.mode, Into::into),
        rater_design: config.reference.rater_design,
        level: config.stats.level,
        bootstrap: config.stats.bootstrap,
        seed: config.stats.seed.unwrap_or(0),
        prevalence: None,
        by: None,
        items: false,
    }
}
