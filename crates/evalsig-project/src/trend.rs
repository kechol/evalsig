//! `evalsig trend <run> --by week`: a run's metrics per period of `occurred_at`, each
//! period compared with a baseline period as independent samples (metrics spec §7.3).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use evalsig_core::compare::{self, Drift, Options};
use evalsig_core::config::Config;
use evalsig_core::records::{Judgment, Label};
use evalsig_core::report;
use evalsig_core::time::{Instant, Period};
use serde_json::{Value, json};

use crate::Error;
use crate::analysis::{self, Analysis};
use crate::data::{self, Data};
use crate::envelope::{Envelope, Output, Schema, Verdict, VerdictCode, Warning, round_numbers};
use crate::exit::Exit;
use crate::project::Project;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum By {
    Day,
    #[default]
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

#[derive(Debug, Clone, Default)]
pub struct Request {
    /// The run to follow over time.
    pub run: String,
    pub analysis: Analysis,
    /// The period length.
    pub by: By,
    /// Only items that occurred at or after this RFC 3339 time.
    pub since: Option<String>,
    /// The period the others are compared with (default: the first).
    pub baseline_period: Option<String>,
}

pub fn run(root: &Path, args: &Request) -> Result<Output, Error> {
    let project = Project::open(root)?;
    compute(&data::load(&project)?, &project.config()?, args)
}

/// The trend of data already loaded, under this configuration.
pub fn compute(d: &Data, config: &Config, args: &Request) -> Result<Output, Error> {
    args.analysis.without_proxy("trend")?;
    let run = d.run(&args.run)?;
    let items = d.items();
    let (all, _) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = all.iter().collect();
    let settings = analysis::settings(config, &args.analysis);
    let since = args
        .since
        .as_deref()
        .map(Instant::parse)
        .transpose()
        .map_err(|e| Error::usage(e.to_string()))?;
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
                Error::usage(format!("no judged items fall in period {p:?}")).with_hint(format!(
                    "periods: {}",
                    periods.keys().cloned().collect::<Vec<_>>().join(", ")
                )),
            );
        }
        None => periods.keys().next().cloned().ok_or_else(|| {
            Error::new(
                "insufficient_data",
                "no judged item has occurred_at",
                Exit::InsufficientData,
            )
            .with_hint(
                "give each item the time it arose as occurred_at and import the records again",
            )
        })?,
    };
    let prepare = |js: &Vec<&Judgment>| {
        report::prepare(run, js, &items, &labels, &settings).map_err(Error::report)
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
            .map_err(Error::report)?;
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
        Schema::Trend,
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
