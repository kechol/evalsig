//! `evalsig report <run>`: the default-tier metrics of one run.

use std::path::Path;

use evalsig_core::records::Label;
use evalsig_core::report;
use evalsig_core::validity;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use serde_json::json;

use crate::analysis::{self, AnalysisArgs};
use crate::checks::{self, ArmArgs, DatasetArgs};
use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Warning, config_hash, round_numbers};
use crate::project::{Project, io_error};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The run to report.
    pub run: String,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
    /// Also report each value of this stratum.
    #[arg(long, value_name = "STRATUM")]
    pub by: Option<String>,
    /// Include one row per item.
    #[arg(long)]
    pub items: bool,
    #[command(flatten)]
    pub checks: DatasetArgs,
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let run = d.run(&args.run)?;
    let (labels, undated_skipped) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = labels.iter().collect();
    let mut settings = analysis::settings(&config, &args.analysis);
    settings.prevalence = config.cost.as_ref().map(|c| c.prevalence);
    settings.by = args.by.clone();
    settings.items = args.items;
    let report = report::build(run, &d.judgments(&args.run), &d.items(), &labels, &settings)
        .map_err(|e| CliError::new("report", e.to_string(), Exit::Rejected))?;
    let findings = validity::dataset(
        &d.records,
        &checks::overrides(args.checks, ArmArgs::default()),
    );
    let results = round_numbers(serde_json::to_value(&report).expect("report serializes"));
    let output_hash = ContentHash::blake3(&to_canonical_bytes(&results).map_err(CliError::store)?);
    let cfg = config_hash(&config);
    let mut envelope = Envelope::new("evalsig.report/1", results);
    if undated_skipped > 0 {
        envelope.warnings.push(Warning {
            code: "undated_label_revisions".into(),
            message: format!("{undated_skipped} labels have newer undated revisions that --labels-as-of cannot place in time"),
        });
    }
    envelope.provenance.snapshots = d.snapshots.iter().map(ToString::to_string).collect();
    envelope.provenance.config = Some(cfg.to_string());
    envelope.provenance.seed = Some(settings.seed);
    let summary = json!({
        "run": args.run,
        "task": settings.task,
        "estimand": settings.estimand.to_string(),
        "seed": settings.seed,
    });
    // Save the report before the ledger names its hash, so a recorded output always
    // exists; the saved copy carries everything but the ledger position.
    let path = project
        .dir
        .join("reports")
        .join(format!("{}.json", output_hash.hex()));
    let mut saved = serde_json::to_string_pretty(&envelope).expect("envelope serializes");
    saved.push('\n');
    std::fs::write(&path, saved).map_err(|e| io_error(&path, &e))?;
    let inputs = envelope
        .provenance
        .snapshots
        .iter()
        .cloned()
        .chain([cfg.to_string()])
        .collect();
    project.record(
        &mut envelope,
        Kind::Report,
        inputs,
        vec![output_hash.to_string()],
        summary.as_object().expect("object").clone(),
    )?;
    Ok(checks::apply(envelope, &findings))
}
