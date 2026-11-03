//! `evalsig trend <run> --by week`: a run's metrics per period of `occurred_at`, each
//! period compared with a baseline period as independent samples (metrics spec §7.3).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use evalsig_core::compare::{self, Drift, Options};
use evalsig_core::records::{Judgment, Label};
use evalsig_core::report;
use evalsig_core::time::{Instant, Period};
use serde_json::{Value, json};

use crate::analysis::{self, AnalysisArgs};
use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning, round_numbers};
use crate::project::Project;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum By {
    Day,
    Week,
    Month,
}

impl From<By> for Period {
    fn from(b: By) -> Self {
        match b {
            By::Day => Period::Day,
            By::Week => Period::Week,
            By::Month => Period::Month,
        }
    }
}

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The run to follow over time.
    pub run: String,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
    /// The period length.
    #[arg(long, value_enum, default_value_t = By::Week)]
    pub by: By,
    /// Only items that occurred at or after this RFC 3339 time.
    #[arg(long, value_name = "TIME")]
    pub since: Option<String>,
    /// The period the others are compared with (default: the first).
    #[arg(long, value_name = "PERIOD")]
    pub baseline_period: Option<String>,
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    args.analysis.without_proxy("trend")?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let run = d.run(&args.run)?;
    let items = d.items();
    let (all, _) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = all.iter().collect();
    let settings = analysis::settings(&config, &args.analysis);
    let since = args
        .since
        .as_deref()
        .map(Instant::parse)
        .transpose()
        .map_err(|e| CliError::usage(e.to_string()))?;
    let mut periods: BTreeMap<String, Vec<&Judgment>> = BTreeMap::new();
    let mut undated = BTreeSet::new();
    for j in d.judgments(&args.run) {
        let at = items
            .get(j.item_id.as_str())
            .and_then(|i| i.occurred_at.as_deref())
            .map(Instant::parse);
        match at {
            Some(Ok(t)) if since.is_none_or(|s| t >= s) => {
                periods.entry(t.period(args.by.into())).or_default().push(j);
            }
            Some(Ok(_)) => {}
            _ => {
                undated.insert(j.item_id.as_str());
            }
        }
    }
    let baseline = match &args.baseline_period {
        Some(p) if periods.contains_key(p) => p.clone(),
        Some(p) => {
            return Err(
                CliError::usage(format!("no judged items fall in period {p:?}")).with_hint(
                    format!(
                        "periods: {}",
                        periods.keys().cloned().collect::<Vec<_>>().join(", ")
                    ),
                ),
            );
        }
        None => periods.keys().next().cloned().ok_or_else(|| {
            CliError::new(
                "insufficient_data",
                "no judged item has occurred_at",
                Exit::InsufficientData,
            )
        })?,
    };
    let prepare = |js: &Vec<&Judgment>| {
        report::prepare(run, js, &items, &labels, &settings).map_err(CliError::report)
    };
    let base = prepare(&periods[&baseline])?;
    let options = Options {
        level: settings.level,
        bootstrap: settings.bootstrap,
        seed: settings.seed,
        min_effect: config.compare.min_effect.clone(),
        excluded: BTreeSet::new(),
        scope: None,
    };
    let mut rows = Vec::new();
    let mut primary_status = None;
    for (period, js) in &periods {
        // Each period is prepared once, for its report and its comparison.
        let p = if *period == baseline {
            None
        } else {
            Some(prepare(js)?)
        };
        let r = report::from_prepared(p.as_ref().unwrap_or(&base), &items, &labels)
            .map_err(CliError::report)?;
        let vs: Vec<Value> = match &p {
            None => Vec::new(),
            Some(p) => {
                let deltas = compare::unpaired(&base, p, &options).metrics;
                if let Some(first) = deltas.first() {
                    primary_status = Some((first.metric, compare::drift(first)));
                }
                deltas
                    .iter()
                    .map(|m| {
                        json!({
                            "metric": m.metric,
                            "delta": m.delta,
                            "ci": m.ci,
                            "interval": m.interval,
                            "status": compare::drift(m),
                        })
                    })
                    .collect()
            }
        };
        rows.push(json!({
            "period": period,
            "n_items": r.n_items,
            "metrics": r.metrics,
            "vs_baseline": vs,
        }));
    }
    // The metric the verdict reads: the first one the periods are compared on.
    let primary = primary_status.map(|(m, _)| m.to_owned()).or_else(|| {
        rows.first()
            .and_then(|r| r["metrics"][0]["metric"].as_str())
            .map(str::to_owned)
    });
    let latest = periods.keys().next_back().cloned();
    let mut envelope = Envelope::new(
        "evalsig.trend/1",
        round_numbers(json!({
            "run": args.run,
            "by": Period::from(args.by),
            "baseline_period": baseline,
            "primary_metric": primary,
            "periods": rows,
            "undated_items": undated.len(),
        })),
    );
    // The verdict is the latest period's status on the primary metric.
    envelope.verdict = if latest.as_deref() == Some(baseline.as_str()) {
        None
    } else {
        primary_status.map(|(metric, s)| {
            let (code, word) = match s {
                Drift::Drifted => (VerdictCode::Drifted, "drifted"),
                Drift::Stable => (VerdictCode::Stable, "stable"),
                Drift::InsufficientData => (VerdictCode::InsufficientData, "undecided"),
            };
            Verdict {
                code,
                text: format!(
                    "{}: the latest period {} is {word} against {baseline}",
                    metric,
                    latest.as_deref().unwrap_or("?"),
                ),
            }
        })
    };
    if !undated.is_empty() {
        envelope.warnings.push(Warning {
            code: "undated_items".into(),
            message: format!(
                "{} judged items have no occurred_at and are left out",
                undated.len()
            ),
        });
    }
    envelope.provenance.snapshots = d.snapshot_ids();
    Ok(Output::success(envelope))
}
