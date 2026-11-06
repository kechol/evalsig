//! `evalsig report <run>`: the default-tier metrics of one run.

use std::path::Path;

use evalsig_core::checks::Finding;
use evalsig_core::config::Config;
use evalsig_core::records::Label;
use evalsig_core::report::Sections;
use evalsig_core::validity;
use evalsig_core::{kcurve, report};
use serde_json::{Value, json};

use crate::Error;
use crate::analysis::{self, Analysis};
use crate::checks::{self, ArmChecks, DatasetChecks};
use crate::data::{self, Data};
use crate::envelope::{Envelope, Output, Warning, round_numbers};
use crate::project::Project;
use crate::provenance;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Request {
    /// The run to report.
    pub run: String,
    pub analysis: Analysis,
    /// Also report each value of this stratum. `reference_agreement` splits the items
    /// two or more human raters labelled into `unanimous` and `split`.
    pub by: Option<String>,
    /// Include one row per item.
    pub items: bool,
    /// Also report majority@k, any@k and all@k for k = 1 … the fewest replicates of
    /// any item, with the expected cost per item.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub k_curve: bool,
    /// Add each feature's likelihood ratio and class.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub features: bool,
    /// With --reference human: correct the human-scale value with this model rater's
    /// labels on the items humans did not label (PPI++).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    pub checks: DatasetChecks,
}

/// What a report computes from data and configuration alone; `verify` repeats it.
pub struct Computed {
    pub results: Value,
    pub findings: Vec<Finding>,
    pub warnings: Vec<Warning>,
    pub seed: u64,
}

pub fn compute(d: &Data, config: &Config, args: &Request) -> Result<Computed, Error> {
    let run = d.run(&args.run)?;
    let (labels, undated_skipped) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = labels.iter().collect();
    let mut settings = analysis::settings(config, &args.analysis);
    settings.prevalence = config.cost.as_ref().map(|c| c.prevalence);
    settings.sections = Sections {
        by: args.by.clone(),
        items: args.items,
        detail: true,
        features: args.features,
    };
    let human = matches!(
        args.analysis.reference,
        Some(analysis::ReferenceChoice::Human)
    );
    if human != args.proxy.is_some() {
        return Err(Error::usage(
            "--reference human and --proxy <rater> go together",
        ));
    }
    settings.proxy = args.proxy.clone();
    let judgments = d.judgments(&args.run);
    let items = d.items();
    let report =
        report::build(run, &judgments, &items, &labels, &settings).map_err(Error::report)?;
    let mut results = serde_json::to_value(&report).expect("report serializes");
    if args.k_curve {
        let curve =
            kcurve::build(run, &judgments, &items, &labels, &settings).map_err(Error::report)?;
        results["k_curve"] = serde_json::to_value(curve).expect("curves serialize");
    }
    let findings = validity::dataset(
        &d.records,
        &checks::overrides(args.checks, ArmChecks::default()),
    );
    let mut warnings = Vec::new();
    if undated_skipped > 0 {
        warnings.push(Warning {
            code: "undated_label_revisions".into(),
            message: format!("{undated_skipped} labels have newer undated revisions that --labels-as-of cannot place in time"),
        });
    }
    Ok(Computed {
        results: round_numbers(results),
        findings,
        warnings,
        seed: settings.seed,
    })
}

pub fn run(root: &Path, args: &Request) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let c = compute(&d, &config, args)?;
    let mut envelope = Envelope::new("evalsig.report/1", c.results);
    envelope.warnings = c.warnings;
    let summary = json!({
        "run": args.run,
        "task": envelope.results["task"],
        "estimand": envelope.results["estimand"],
        "seed": c.seed,
    });
    let request = provenance::Request::Report(args.clone());
    provenance::finish(
        &project,
        &config,
        &d,
        &mut envelope,
        c.seed,
        &request,
        summary,
    )?;
    Ok(checks::apply(envelope, &c.findings))
}
