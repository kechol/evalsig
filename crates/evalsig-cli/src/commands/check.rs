//! `evalsig check <check.toml> --run <id>`: a non-preregistered check of one run
//! against interval-based absolute criteria (D-021, SPEC §3.2, §10.1).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use evalsig_core::criteria::{self, Condition};
use evalsig_core::gate;
use evalsig_core::interval::{clopper_pearson, clopper_pearson_upper};
use evalsig_core::missing::Side;
use evalsig_core::records::{Judgment, Label};
use evalsig_core::report::{self, Report};
use evalsig_core::validity;
use serde_json::json;

use evalsig_project::analysis::{self, Analysis};
use evalsig_project::checks::{self, ArmChecks};

use crate::args::{AnalysisArgs, DatasetArgs};
use evalsig_project::Error;
use evalsig_project::data;
use evalsig_project::envelope::{Envelope, Output, Schema, Verdict, VerdictCode, round_numbers};
use evalsig_project::exit::Exit;
use evalsig_project::project::{Project, io_error};

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

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    Analysis::from(args.analysis.clone()).without_proxy("check")?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let text = std::fs::read_to_string(&args.file).map_err(|e| io_error(&args.file, &e))?;
    let file = criteria::parse(&text)
        .map_err(|e| Error::invalid_conditions(format!("{}: {e}", args.file.display())))?;
    let d = data::load(&project)?;
    let run = d.run(&args.run)?;
    if let Some(g) = &file.grader {
        let actual = run.grader.as_ref();
        let matches = actual.and_then(|a| a.id.as_deref()) == Some(g.id.as_str())
            && g.version
                .as_ref()
                .is_none_or(|v| actual.and_then(|a| a.version.as_ref()) == Some(v));
        if !matches {
            return Err(Error::invalid_conditions(format!(
                "run {} was graded by {:?}, not {:?}",
                args.run,
                actual.and_then(|a| a.id.clone()),
                g.id
            )));
        }
    }
    let (all, _) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = all.iter().collect();
    let mut settings = analysis::settings(&config, &args.analysis.clone().into());
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
    // One report per (interval level, score threshold) the conditions need.
    let key = |c: &Condition| {
        (
            c.interval_level(settings.level).to_bits(),
            c.threshold.map(f64::to_bits),
        )
    };
    let mut reports: BTreeMap<(u64, Option<u64>), Report> = BTreeMap::new();
    for c in &file.conditions {
        if let std::collections::btree_map::Entry::Vacant(slot) = reports.entry(key(c)) {
            let s = report::Settings {
                level: c.interval_level(settings.level),
                threshold: c.threshold.or(settings.threshold),
                exact_rates: true,
                ..settings.clone()
            };
            slot.insert(
                report::build(run, &judgments, &items, &labels, &s).map_err(Error::report)?,
            );
        }
    }
    let n_items = reports.values().next().map_or(0, |r| r.n_items);
    // `replicates`: every item needs at least that many generation replicates.
    let mut per_item: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    for j in &judgments {
        per_item
            .entry(j.item_id.as_str())
            .or_default()
            .insert(j.gen_replicate);
    }
    let short = file.replicates.map_or(0, |k| {
        per_item.values().filter(|g| (g.len() as u32) < k).count()
    });
    let enough = file.min_items.is_none_or(|m| n_items as u64 >= m) && short == 0;
    let mut rows = Vec::new();
    let mut outcomes = Vec::new();
    for c in &file.conditions {
        let (estimate, interval) = condition_value(
            c,
            &reports[&key(c)],
            &judgments,
            c.level.unwrap_or(settings.level),
        )?;
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
        Schema::Check,
        round_numbers(json!({
            "name": file.name,
            "run": args.run,
            "preregistered": false,
            "level": settings.level,
            "n_items": n_items,
            "min_items": file.min_items,
            "replicates": file.replicates,
            "items_with_fewer_replicates": short,
            "conditions": rows,
        })),
    );
    envelope.verdict = Some(Verdict {
        code,
        text: format!("{summary} (a non-preregistered check, not a gate)"),
    });
    envelope.provenance.snapshots = d.snapshot_ids();
    let findings = validity::dataset(
        &d.records,
        &checks::overrides(args.checks.into(), ArmChecks::default()),
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
    r: &Report,
    judgments: &[&Judgment],
    level: f64,
) -> Result<Measured, Error> {
    if c.metric == "missing_rate" {
        let (k, n) = gate::missing_counts(judgments, |j| {
            j.missing
                .as_ref()
                .is_some_and(|m| m.reason.side() == Side::System)
        });
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
            Error::invalid_conditions(format!(
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
