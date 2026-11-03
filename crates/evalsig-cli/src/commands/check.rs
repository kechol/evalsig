//! `evalsig check <check.toml> --run <id>`: a non-preregistered check of one run
//! against interval-based absolute criteria (D-021, SPEC §3.2, §10.1).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use evalsig_core::criteria::{self, Condition};
use evalsig_core::interval::{clopper_pearson, clopper_pearson_upper};
use evalsig_core::missing::Side;
use evalsig_core::records::Label;
use evalsig_core::report::{self, Report, ReportError};
use evalsig_core::validity;
use serde_json::json;

use crate::analysis::{self, AnalysisArgs};
use crate::checks::{self, ArmArgs, DatasetArgs};
use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, round_numbers};
use crate::project::{Project, io_error};

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The conditions (a subset of gate.toml).
    pub file: PathBuf,
    /// The run to check.
    #[arg(long)]
    pub run: String,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
    #[command(flatten)]
    pub checks: DatasetArgs,
}

fn rejected(message: impl Into<String>) -> CliError {
    CliError::new("invalid_conditions", message, Exit::Rejected)
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let text = std::fs::read_to_string(&args.file).map_err(|e| io_error(&args.file, &e))?;
    let file =
        criteria::parse(&text).map_err(|e| rejected(format!("{}: {e}", args.file.display())))?;
    let d = data::load(&project)?;
    let run = d.run(&args.run)?;
    if let Some(g) = &file.grader {
        let actual = run.grader.as_ref();
        let matches = actual.and_then(|a| a.id.as_deref()) == Some(g.id.as_str())
            && g.version
                .as_ref()
                .is_none_or(|v| actual.and_then(|a| a.version.as_ref()) == Some(v));
        if !matches {
            return Err(rejected(format!(
                "run {} was graded by {:?}, not {:?}",
                args.run,
                actual.and_then(|a| a.id.clone()),
                g.id
            )));
        }
    }
    let (all, _) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = all.iter().collect();
    let mut settings = analysis::settings(&config, &args.analysis);
    if let Some(e) = file.estimand {
        settings.estimand = e;
    }
    settings.positive = file.positive.or(settings.positive);
    settings.decision = file.decision.or(settings.decision);
    if let Some(r) = &file.reference {
        settings.reference = r.mode;
        settings.rater_design = r.rater_design;
    }
    let judgments = d.judgments(&args.run);
    let items = d.items();
    let failed = |e: ReportError| CliError::new("report", e.to_string(), Exit::Rejected);
    // One report per interval level the conditions need.
    let mut reports: BTreeMap<u64, Report> = BTreeMap::new();
    let mut report_at = |level: f64| -> Result<Report, CliError> {
        if let Some(r) = reports.get(&level.to_bits()) {
            return Ok(r.clone());
        }
        let s = report::Settings {
            level,
            ..settings.clone()
        };
        let r = report::build(run, &judgments, &items, &labels, &s).map_err(failed)?;
        reports.insert(level.to_bits(), r.clone());
        Ok(r)
    };
    let base = report_at(settings.level)?;
    let enough = file.min_items.is_none_or(|m| base.n_items as u64 >= m);
    let mut rows = Vec::new();
    let mut outcomes = Vec::new();
    for c in &file.conditions {
        let (estimate, interval) = condition_value(c, &mut report_at, settings.level)?;
        let judged = if enough { c.judge(interval) } else { None };
        outcomes.push(judged.map(|(_, passed)| passed));
        rows.push(json!({
            "metric": c.metric,
            "rule": c.rule,
            "value": c.value,
            "sided": c.sided,
            "estimate": estimate,
            "bound": judged.map(|(b, _)| b),
            "passed": judged.map(|(_, p)| p),
        }));
    }
    let (code, exit, summary) = if outcomes.contains(&Some(false)) {
        (VerdictCode::Fail, Exit::Failed, "a condition fails")
    } else if outcomes.contains(&None) {
        (
            VerdictCode::InsufficientData,
            Exit::InsufficientData,
            "a condition could not be decided",
        )
    } else {
        (VerdictCode::Pass, Exit::Success, "every condition holds")
    };
    let mut envelope = Envelope::new(
        "evalsig.check/1",
        round_numbers(json!({
            "name": file.name,
            "run": args.run,
            "preregistered": false,
            "level": settings.level,
            "n_items": base.n_items,
            "min_items": file.min_items,
            "conditions": rows,
        })),
    );
    envelope.verdict = Some(Verdict {
        code,
        text: format!("{summary} (a non-preregistered check, not a gate)"),
    });
    envelope.provenance.snapshots = d.snapshots.iter().map(ToString::to_string).collect();
    let findings = validity::dataset(
        &d.records,
        &checks::overrides(args.checks, ArmArgs::default()),
    );
    let out = checks::apply(envelope, &findings);
    Ok(if out.exit == Exit::Success {
        Output { exit, ..out }
    } else {
        out
    })
}

/// A point value and its interval.
type Measured = (Option<f64>, Option<(f64, f64)>);

/// A condition's point value and the interval at the level its sidedness needs.
fn condition_value(
    c: &Condition,
    report_at: &mut dyn FnMut(f64) -> Result<Report, CliError>,
    level: f64,
) -> Result<Measured, CliError> {
    let r = report_at(c.interval_level(level))?;
    if c.metric == "missing_rate" {
        let system: u64 = r
            .missing_by_reason
            .iter()
            .filter(|(reason, _)| reason.side() == Side::System)
            .map(|(_, n)| n)
            .sum();
        let (k, n) = (system as f64, r.n_judgments as f64);
        let interval = (n > 0.0).then(|| match c.sided {
            criteria::Sided::One => (0.0, clopper_pearson_upper(k, n, level)),
            criteria::Sided::Two => clopper_pearson(k, n, level),
        });
        return Ok(((n > 0.0).then(|| k / n), interval));
    }
    let m = r
        .metrics
        .iter()
        .find(|m| m.metric == c.metric)
        .ok_or_else(|| {
            rejected(format!(
                "metric {:?} is not available for this run; available: {}, missing_rate",
                c.metric,
                r.metrics
                    .iter()
                    .map(|m| m.metric)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
    Ok((m.value, m.ci))
}
