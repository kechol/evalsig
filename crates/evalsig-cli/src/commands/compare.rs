//! `evalsig compare <baseline> <candidate>...`: paired comparison and verdict.

use std::collections::BTreeMap;
use std::path::Path;

use evalsig_core::compare::{self, Comparison, Options};
use evalsig_core::config::Estimand;
use evalsig_core::paired::{self, Verdict};
use evalsig_core::records::{DEFAULT_TASK, Item, Judgment, Record, Run};
use evalsig_core::report::{self, Prepared, Settings};
use evalsig_core::validity;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind, LedgerSide, NewEntry};
use serde_json::{Value, json};

use crate::checks::{self, ArmArgs, DatasetArgs};
use crate::commands::report::{ReferenceArg, config_hash, round_numbers, select_labels};
use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Next, Output, Verdict as EnvelopeVerdict, VerdictCode};
use crate::project::{Project, actor, now};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The baseline run.
    pub baseline: String,
    /// One or more candidate runs.
    #[arg(required = true)]
    pub candidates: Vec<String>,
    /// The judgment task.
    #[arg(long, default_value = DEFAULT_TASK)]
    pub task: String,
    /// The metric that decides the verdict (default: the first the runs support).
    #[arg(long)]
    pub metric: Option<String>,
    /// The estimand: single_shot, majority@k, any@k or all@k (default from the configuration).
    #[arg(long)]
    pub estimand: Option<Estimand>,
    /// Decision threshold for score outputs; for score runs, also compares the share at or above it.
    #[arg(long)]
    pub threshold: Option<f64>,
    /// Use the labels as they were at this RFC 3339 time.
    #[arg(long, value_name = "TIME")]
    pub labels_as_of: Option<String>,
    /// How several raters' labels become the reference (default from the configuration).
    #[arg(long, value_enum)]
    pub reference: Option<ReferenceArg>,
    /// Include one row per paired item (`--format jsonl` prints only these rows).
    #[arg(long)]
    pub items: bool,
    /// Do not record the comparison in the ledger.
    #[arg(long)]
    pub scratch: bool,
    #[command(flatten)]
    pub dataset: DatasetArgs,
    #[command(flatten)]
    pub arms: ArmArgs,
}

fn find_run<'a>(records: &'a [Record], id: &str) -> Result<&'a Run, CliError> {
    records
        .iter()
        .find_map(|r| match r {
            Record::Run(run) if run.run_id == id => Some(run),
            _ => None,
        })
        .ok_or_else(|| {
            CliError::usage(format!("no run named {id:?}"))
                .with_hint("list runs with `evalsig runs`")
        })
}

fn judgments_of<'a>(records: &'a [Record], id: &str) -> Vec<&'a Judgment> {
    records
        .iter()
        .filter_map(|r| match r {
            Record::Judgment(j) if j.run_id == id => Some(j),
            _ => None,
        })
        .collect()
}

fn verdict_text(metric: &str, d: &compare::MetricDelta, level: f64) -> String {
    let pct = (level * 100.0).round();
    match (d.delta, d.ci) {
        (Some(delta), Some((lo, hi))) => {
            let what = match d.verdict {
                Verdict::Improved => "the candidate improved",
                Verdict::Worsened => "the candidate is worse",
                Verdict::Negligible => "the difference is too small to matter",
                Verdict::Indistinguishable => "the arms cannot be told apart",
                Verdict::InsufficientData => "there are too few items to decide",
            };
            format!(
                "{metric}: difference {delta:+.4} (better is positive; {pct}% interval {lo:+.4} to {hi:+.4}), so {what}."
            )
        }
        _ => format!("{metric}: too few paired items to compare."),
    }
}

fn comparison_json(baseline: &str, candidate: &str, c: &Comparison, primary: &str) -> Value {
    json!({
        "arms": { "baseline": baseline, "candidate": candidate },
        "primary_metric": primary,
        "n_used": c.n_used,
        "n_unpaired": c.n_unpaired,
        "n_input_changed": 0,
        "metrics": c.metrics,
        "transition": c.transition,
    })
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let items: BTreeMap<&str, &Item> = d
        .records
        .iter()
        .filter_map(|r| match r {
            Record::Item(i) => Some((i.item_id.as_str(), i)),
            _ => None,
        })
        .collect();
    let (labels, _) = select_labels(&d.records, args.labels_as_of.as_deref())?;
    let settings = Settings {
        task: args.task.clone(),
        positive: config.positive().for_task(&args.task).copied(),
        decision: config.decision().for_task(&args.task).copied(),
        threshold: args.threshold,
        estimand: args.estimand.unwrap_or(config.stats.estimand),
        missing: config.missing_policy(),
        reference: args.reference.map_or(config.reference.mode, Into::into),
        level: config.stats.level,
        bootstrap: config.stats.bootstrap,
        seed: config.stats.seed.unwrap_or(0),
        prevalence: None,
        by: None,
        items: false,
    };
    let prepare = |id: &str| -> Result<(&Run, Prepared<'_>), CliError> {
        let run = find_run(&d.records, id)?;
        let p = report::prepare(
            run,
            &judgments_of(&d.records, id),
            &items,
            &labels,
            &settings,
        )
        .map_err(|e| CliError::new("report", e.to_string(), Exit::Rejected))?;
        Ok((run, p))
    };
    let (base_run, base) = prepare(&args.baseline)?;
    let overrides = checks::overrides(args.dataset, args.arms);
    let mut findings = validity::dataset(&d.records, &overrides);
    let options = Options {
        level: settings.level,
        bootstrap: settings.bootstrap,
        seed: settings.seed,
        min_effect: config.compare.min_effect.clone(),
    };
    let mut comparisons = Vec::new();
    for cand_id in &args.candidates {
        let (cand_run, cand) = prepare(cand_id)?;
        if cand.semantics() != base.semantics() {
            return Err(CliError::new(
                "incomparable",
                format!(
                    "{} and {cand_id} have different value semantics",
                    args.baseline
                ),
                Exit::Rejected,
            ));
        }
        findings.extend(validity::mixed_grader(
            base_run.grader.as_ref(),
            cand_run.grader.as_ref(),
            &overrides,
        ));
        findings.extend(validity::missing_rate_differs(
            base.system_missing(),
            cand.system_missing(),
            &overrides,
        ));
        let c = compare::compare(&base, &cand, &options);
        if c.n_unpaired > 0 {
            findings.push(
                evalsig_core::checks::Finding::new(
                    "unpaired_items",
                    format!(
                        "{} items are in only one arm; compared on the {} common items",
                        c.n_unpaired, c.n_used
                    ),
                    &overrides,
                )
                .with_count(c.n_unpaired),
            );
        }
        comparisons.push((cand_id.clone(), cand, c));
    }
    let primary = match &args.metric {
        Some(m) => {
            let known: Vec<&str> = comparisons[0].2.metrics.iter().map(|x| x.metric).collect();
            if !known.contains(&m.as_str()) {
                return Err(CliError::usage(format!(
                    "metric {m:?} is not available for these runs"
                ))
                .with_hint(format!("available: {}", known.join(", "))));
            }
            m.clone()
        }
        None => comparisons[0]
            .2
            .metrics
            .first()
            .map(|x| x.metric.to_owned())
            .unwrap_or_default(),
    };
    let pick = |c: &Comparison| c.metrics.iter().find(|m| m.metric == primary).cloned();
    let item_rows = |cand: &Prepared<'_>| -> Vec<Value> {
        let (ja, jb) = (base.judged(), cand.judged());
        ja.iter()
            .filter_map(|(item, (va, s))| {
                jb.get(item).map(|(vb, _)| json!({"item_id": item, "reference": s, "baseline": va, "candidate": vb}))
            })
            .collect()
    };
    let common = json!({
        "value_semantics": base.semantics(),
        "positive": base.positive(),
        "decision": base.rule(),
        "estimand": settings.estimand.to_string(),
        "reference": base.positive().map(|_| json!({"mode": settings.reference, "raters": base.raters()})),
        "n_missing": base.missing_by_reason().values().sum::<u64>()
            + comparisons.iter().map(|(_, p, _)| p.missing_by_reason().values().sum::<u64>()).sum::<u64>(),
        "missing_policy": settings.missing.table(),
    });
    let (mut results, verdict) = if comparisons.len() == 1 {
        let (cand_id, cand, c) = &comparisons[0];
        let mut r = comparison_json(&args.baseline, cand_id, c, &primary);
        merge(&mut r, &common);
        if args.items {
            r["items"] = Value::Array(item_rows(cand));
        }
        (r, pick(c))
    } else {
        let ps: Vec<f64> = comparisons
            .iter()
            .filter_map(|(_, _, c)| pick(c).and_then(|m| m.p_value))
            .collect();
        let adjusted = (ps.len() == comparisons.len())
            .then(|| paired::holm(&ps))
            .flatten();
        let list: Vec<Value> = comparisons
            .iter()
            .map(|(id, cand, c)| {
                let mut v = comparison_json(&args.baseline, id, c, &primary);
                if args.items {
                    v["items"] = Value::Array(item_rows(cand));
                }
                v
            })
            .collect();
        let mut r = json!({
            "arms": { "baseline": args.baseline, "candidates": args.candidates },
            "primary_metric": primary,
            "comparisons": list,
            "holm": { "metric": primary, "adjusted_p": adjusted },
            "selected_after_comparison": true,
        });
        merge(&mut r, &common);
        // The overall verdict is the best candidate's on the primary metric.
        let best = comparisons
            .iter()
            .filter_map(|(_, _, c)| pick(c))
            .max_by(|x, y| {
                x.delta
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&y.delta.unwrap_or(f64::NEG_INFINITY))
            });
        (r, best)
    };
    results = round_numbers(results);
    let output_hash = ContentHash::blake3(&to_canonical_bytes(&results).map_err(CliError::store)?);
    let cfg = config_hash(&config);
    let mut envelope = Envelope::new("evalsig.compare/1", results);
    if let Some(m) = &verdict {
        envelope.verdict = Some(EnvelopeVerdict {
            code: VerdictCode::from(m.verdict),
            text: verdict_text(&primary, m, settings.level),
        });
        if matches!(
            m.verdict,
            Verdict::Indistinguishable | Verdict::InsufficientData
        ) {
            envelope.next.push(Next {
                command: format!("evalsig power --metric {primary} --paired"),
                why: "estimate how many items it takes to detect the minimum effect".into(),
            });
        }
    }
    if comparisons.len() > 1 {
        envelope.warnings.push(crate::output::Warning {
            code: "selected_after_comparison".into(),
            message:
                "the best of several candidates is biased upwards; confirm it on another split"
                    .into(),
        });
    }
    envelope.provenance.snapshots = d.snapshots.iter().map(ToString::to_string).collect();
    envelope.provenance.config = Some(cfg.to_string());
    envelope.provenance.seed = Some(settings.seed);
    if !args.scratch {
        let mut summary = serde_json::Map::new();
        summary.insert("baseline".into(), json!(args.baseline));
        summary.insert("candidates".into(), json!(args.candidates));
        summary.insert("metric".into(), json!(primary));
        summary.insert("verdict".into(), json!(verdict.as_ref().map(|m| m.verdict)));
        let entry = ledger::append(
            &project.ledger_path(),
            LedgerSide::Agent,
            Kind::Compare,
            NewEntry {
                ts: now(),
                actor: actor(),
                inputs: envelope
                    .provenance
                    .snapshots
                    .iter()
                    .cloned()
                    .chain([cfg.to_string()])
                    .collect(),
                outputs: vec![output_hash.to_string()],
                summary,
                tool: crate::tool_version(),
            },
        )
        .map_err(CliError::store)?;
        envelope.provenance.ledger = Some("agent".into());
        envelope.provenance.ledger_seq = Some(entry.seq);
    }
    Ok(checks::apply(envelope, &findings))
}

fn merge(target: &mut Value, extra: &Value) {
    if let (Value::Object(t), Value::Object(e)) = (target, extra) {
        for (k, v) in e {
            t.insert(k.clone(), v.clone());
        }
    }
}
