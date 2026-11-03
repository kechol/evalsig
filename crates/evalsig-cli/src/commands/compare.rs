//! `evalsig compare <baseline> <candidate>...`: paired comparison and verdict.

use std::path::Path;

use std::collections::{BTreeMap, BTreeSet};

use evalsig_core::checks::{Finding, Overrides};
use evalsig_core::compare::{self, Baseline, Comparison, Options};
use evalsig_core::config::Config;
use evalsig_core::paired::Verdict;
use evalsig_core::preference;
use evalsig_core::records::{Item, Label, Preference, Record, Run};
use evalsig_core::report::{self, Prepared};
use evalsig_core::validity;
use serde_json::{Value, json};

use crate::analysis::{self, AnalysisArgs};
use crate::checks::{self, ArmArgs, DatasetArgs};
use crate::data::{self, Data};
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{
    Envelope, Output, Verdict as EnvelopeVerdict, VerdictCode, Warning, round_numbers,
};
use crate::project::Project;
use crate::provenance::{self, Request};

#[derive(Debug, Clone, clap::Args, serde::Serialize, serde::Deserialize)]
pub struct Args {
    /// The baseline run.
    pub baseline: String,
    /// One or more candidate runs.
    #[arg(required = true)]
    pub candidates: Vec<String>,
    /// The metric that decides the verdict (default: the first the runs support).
    #[arg(long)]
    pub metric: Option<String>,
    #[command(flatten)]
    pub analysis: AnalysisArgs,
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

fn comparison_json(
    baseline: &str,
    candidate: &str,
    c: &Comparison,
    primary: &str,
    n_input_changed: usize,
) -> Value {
    json!({
        "arms": { "baseline": baseline, "candidate": candidate },
        "primary_metric": primary,
        "n_used": c.n_used,
        "n_unpaired": c.n_unpaired,
        "n_input_changed": n_input_changed,
        "metrics": c.metrics,
        "transition": c.transition,
    })
}

/// Items present in both arms whose input differs, by `input_ref` of the item records
/// each arm reads.
fn changed_inputs(
    items: &BTreeMap<&str, &Item>,
    base: &Prepared<'_>,
    cand: &Prepared<'_>,
    overrides: &Overrides,
) -> (BTreeSet<String>, Vec<Finding>) {
    let inputs = |p: &Prepared<'_>| -> BTreeMap<String, Option<String>> {
        p.judged()
            .into_keys()
            .map(|k| {
                let input = items.get(k.as_str()).and_then(|i| i.input_ref.clone());
                (k, input)
            })
            .collect()
    };
    let (a, b) = (inputs(base), inputs(cand));
    let a_ref: BTreeMap<&str, Option<&str>> =
        a.iter().map(|(k, v)| (k.as_str(), v.as_deref())).collect();
    let b_ref: BTreeMap<&str, Option<&str>> =
        b.iter().map(|(k, v)| (k.as_str(), v.as_deref())).collect();
    let (pairing, findings) = validity::pair(&a_ref, &b_ref, overrides);
    let excluded = if overrides.allow_input_change {
        BTreeSet::new()
    } else {
        pairing
            .input_changed
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    };
    // unpaired_items is reported by compare itself; keep only the input finding.
    (
        excluded,
        findings
            .into_iter()
            .filter(|f| f.code == "input_changed")
            .collect(),
    )
}

/// Pairwise preferences between the baseline and a candidate, for the task: the
/// candidate's win rate, the position effects and the raters whose judgments count.
/// `None` when no preference record compares the two runs.
fn preference_json(
    preferences: &[&Preference],
    baseline: &str,
    candidate: &str,
    settings: &report::Settings,
    overrides: &Overrides,
    findings: &mut Vec<Finding>,
) -> Option<Value> {
    let judged = preference::normalise(preferences.iter().copied(), baseline, candidate);
    let win = preference::win_rate(&judged, settings.level, settings.bootstrap, settings.seed)?;
    let bias = preference::position_bias(&judged, settings.level);
    findings.extend(preference::checks(&bias, overrides));
    let raters: BTreeSet<&str> = judged.iter().map(|j| j.rater_id).collect();
    let sources: BTreeSet<String> = preferences
        .iter()
        .filter(|p| raters.contains(p.rater_id.as_str()))
        .filter_map(|p| json!(p.source).as_str().map(str::to_owned))
        .collect();
    Some(json!({
        "win_rate": win,
        "position_bias": bias,
        "raters": raters,
        "sources": sources,
    }))
}

/// What a comparison computes from data and configuration alone; `verify` repeats it.
pub struct Computed {
    pub results: Value,
    pub verdict: Option<EnvelopeVerdict>,
    pub findings: Vec<Finding>,
    pub warnings: Vec<Warning>,
    pub seed: u64,
}

pub fn compute(d: &Data, config: &Config, args: &Args) -> Result<Computed, CliError> {
    let items = d.items();
    let (labels, undated_skipped) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = labels.iter().collect();
    let settings = analysis::settings(config, &args.analysis);
    let prepare = |id: &str| -> Result<(&Run, Prepared<'_>), CliError> {
        let run = d.run(id)?;
        let p = report::prepare(run, &d.judgments(id), &items, &labels, &settings)
            .map_err(|e| CliError::new("report", e.to_string(), Exit::Rejected))?;
        Ok((run, p))
    };
    let (base_run, base) = prepare(&args.baseline)?;
    let baseline = Baseline::new(&base);
    let overrides = checks::overrides(args.dataset, args.arms);
    let mut findings = validity::dataset(&d.records, &overrides);
    let mut comparisons = Vec::new();
    for cand_id in &args.candidates {
        let (cand_run, cand) = prepare(cand_id)?;
        if (cand.semantics(), cand.output_type(), cand.rule())
            != (base.semantics(), base.output_type(), base.rule())
        {
            return Err(CliError::new(
                "incomparable",
                format!(
                    "{} and {cand_id} differ in value semantics, output type or decision rule",
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
        let (excluded, input_findings) = changed_inputs(&items, &base, &cand, &overrides);
        findings.extend(input_findings);
        let options = Options {
            level: settings.level,
            bootstrap: settings.bootstrap,
            seed: settings.seed,
            min_effect: config.compare.min_effect.clone(),
            excluded: excluded.clone(),
        };
        let c = compare::compare(&baseline, &cand, &options);
        if c.n_unpaired > 0 {
            findings.push(
                Finding::new(
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
        comparisons.push((cand_id.clone(), cand, c, excluded.len()));
    }
    let all: Vec<Comparison> = comparisons.iter().map(|(_, _, c, _)| c.clone()).collect();
    let multi = compare::summarize(&all, args.metric.as_deref()).map_err(|available| {
        CliError::usage(format!(
            "metric {:?} is not available for these runs",
            args.metric.as_deref().unwrap_or("")
        ))
        .with_hint(format!("available: {}", available.join(", ")))
    })?;
    let primary = multi.primary;
    let preferences: Vec<&Preference> = d
        .records
        .iter()
        .filter_map(|r| match r {
            Record::Preference(p) if p.task() == settings.task => Some(p),
            _ => None,
        })
        .collect();
    let pref: BTreeMap<&str, Value> = comparisons
        .iter()
        .filter_map(|(id, _, _, _)| {
            preference_json(
                &preferences,
                &args.baseline,
                id,
                &settings,
                &overrides,
                &mut findings,
            )
            .map(|v| (id.as_str(), v))
        })
        .collect();
    let base_judged = base.judged();
    let list: Vec<Value> = comparisons
        .iter()
        .map(|(id, cand, c, changed)| {
            let mut v = comparison_json(&args.baseline, id, c, primary, *changed);
            if let Some(p) = pref.get(id.as_str()) {
                v["preference"] = p.clone();
            }
            if args.items {
                let jb = cand.judged();
                let rows: Vec<Value> = base_judged
                    .iter()
                    .filter_map(|(item, (va, s))| {
                        jb.get(item).map(|(vb, _)| {
                            json!({"item_id": item, "candidate_run": id, "reference": s, "baseline": va, "candidate": vb})
                        })
                    })
                    .collect();
                v["items"] = Value::Array(rows);
            }
            v
        })
        .collect();
    let common = json!({
        "value_semantics": base.semantics(),
        "positive": base.positive(),
        "decision": base.rule(),
        "estimand": settings.estimand.to_string(),
        "reference": base.positive().map(|_| json!({"mode": settings.reference, "raters": base.raters()})),
        "n_missing": base.n_missing() + comparisons.iter().map(|(_, p, _, _)| p.n_missing()).sum::<u64>(),
        "missing_policy": settings.missing.table(),
    });
    let chosen = if list.len() == 1 { Some(0) } else { multi.best };
    let mut results = if list.len() == 1 {
        list.into_iter().next().expect("one comparison")
    } else {
        json!({
            "arms": { "baseline": args.baseline, "candidates": args.candidates },
            "primary_metric": primary,
            "comparisons": list,
            "holm": { "metric": primary, "adjusted_p": multi.holm },
            "selected_after_comparison": true,
        })
    };
    merge(&mut results, &common);
    let verdict = chosen.and_then(|i| {
        comparisons[i]
            .2
            .metrics
            .iter()
            .find(|m| m.metric == primary)
            .cloned()
    });
    let mut warnings = Vec::new();
    if comparisons.len() > 1 {
        warnings.push(Warning {
            code: "selected_after_comparison".into(),
            message:
                "the best of several candidates is biased upwards; confirm it on another split"
                    .into(),
        });
    }
    if undated_skipped > 0 {
        warnings.push(Warning {
            code: "undated_label_revisions".into(),
            message: format!("{undated_skipped} labels have newer undated revisions that --labels-as-of cannot place in time"),
        });
    }
    Ok(Computed {
        results: round_numbers(results),
        verdict: verdict.as_ref().map(|m| EnvelopeVerdict {
            code: VerdictCode::from(m.verdict),
            text: verdict_text(primary, m, settings.level),
        }),
        findings,
        warnings,
        seed: settings.seed,
    })
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let c = compute(&d, &config, args)?;
    let mut envelope = Envelope::new("evalsig.compare/1", c.results);
    envelope.verdict = c.verdict;
    envelope.warnings = c.warnings;
    let request = Request::Compare(args.clone());
    if args.scratch {
        provenance::fill(&project, &config, &d, &mut envelope, c.seed, &request)?;
    } else {
        let summary = json!({
            "baseline": args.baseline,
            "candidates": args.candidates,
            "metric": envelope.results["primary_metric"],
            "verdict": envelope.verdict.as_ref().map(|v| v.code),
        });
        provenance::finish(
            &project,
            &config,
            &d,
            &mut envelope,
            c.seed,
            &request,
            summary,
        )?;
    }
    Ok(checks::apply(envelope, &c.findings))
}

fn merge(target: &mut Value, extra: &Value) {
    if let (Some(t), Some(e)) = (target.as_object_mut(), extra.as_object()) {
        t.extend(e.clone());
    }
}
