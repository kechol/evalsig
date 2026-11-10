//! Options shared by the analysis commands (`report`, `compare`) and the settings
//! they resolve to.

use evalsig_core::config::{Config, Estimand, ReferenceMode};
use evalsig_core::records::DEFAULT_TASK;
use evalsig_core::report::{Sections, Settings};

/// The options shared by the analyses. Their serde names are the recorded form of a
/// request (D-030).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Analysis {
    /// The judgment task.
    pub task: String,
    /// The estimand; the configuration's when absent.
    pub estimand: Option<Estimand>,
    /// Decision threshold for score outputs; for score runs, also the share at or above it.
    pub threshold: Option<f64>,
    /// Use the labels as they were at this RFC 3339 time.
    pub labels_as_of: Option<String>,
    /// How several raters' labels become the reference; the configuration's when absent.
    pub reference: Option<ReferenceChoice>,
}

impl Default for Analysis {
    fn default() -> Self {
        Analysis {
            task: DEFAULT_TASK.to_owned(),
            estimand: None,
            threshold: None,
            labels_as_of: None,
            reference: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceChoice {
    Single,
    Majority,
    Soft,
    Latent,
    /// The human labels, corrected with a model rater's labels (`report --proxy`).
    Human,
}

impl From<ReferenceChoice> for ReferenceMode {
    fn from(r: ReferenceChoice) -> Self {
        match r {
            ReferenceChoice::Single => ReferenceMode::Single,
            ReferenceChoice::Majority => ReferenceMode::Majority,
            ReferenceChoice::Soft => ReferenceMode::Soft,
            ReferenceChoice::Latent => ReferenceMode::Latent,
            // The human labels form the reference; `--proxy` adds the correction.
            ReferenceChoice::Human => ReferenceMode::Soft,
        }
    }
}

impl Analysis {
    /// Refuse `--reference human`, which only `report --proxy` reads.
    pub fn without_proxy(&self, command: &str) -> Result<(), crate::Error> {
        if matches!(self.reference, Some(ReferenceChoice::Human)) {
            return Err(crate::Error::usage(format!(
                "--reference human applies to report --proxy; {command} uses single, majority or soft"
            )));
        }
        Ok(())
    }
}

/// The settings for these options under a configuration; `prevalence`, `by` and
/// `items` are left for the command to set.
pub fn settings(config: &Config, a: &Analysis) -> Settings {
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
        cost: config.cost.clone(),
        sections: Sections::default(),
        exact_rates: false,
        proxy: None,
    }
}
