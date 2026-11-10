//! `evalsig compare <baseline> <candidate>...`: paired comparison and verdict.

use std::path::Path;

use std::collections::{BTreeMap, BTreeSet};

use evalsig_core::checks::{Finding, Overrides};
use evalsig_core::compare::{self, Baseline, Comparison, Options};
use evalsig_core::config::Config;
use evalsig_core::paired::Verdict;
use evalsig_core::preference;
use evalsig_core::records::{Item, Label, Preference, Record, Run, ValueSemantics};
use evalsig_core::replicates;
use evalsig_core::report::{self, Fill, MetricId, Prepared};
use evalsig_core::split::Part;
use evalsig_core::validity;
use evalsig_store::ledger::{Entry, Kind};
use serde_json::{Value, json};

use crate::Error;
use crate::analysis::{self, Analysis};
use crate::checks::{self, ArmChecks, DatasetChecks};
use crate::data::{self, Data};
use crate::envelope::{
    Envelope, Next, Output, Schema, Verdict as EnvelopeVerdict, VerdictCode, Warning, round_numbers,
};
use crate::exit::Exit;
use crate::project::Project;
use crate::provenance::{self, item_set_hash};
use crate::split::{self, Scope};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Request {
    /// The baseline run.
    pub baseline: String,
    /// One or more candidate runs.
    pub candidates: Vec<String>,
    /// The metric that decides the verdict (default: the first the runs support).
    pub metric: Option<String>,
    pub analysis: Analysis,
    /// Include one row per paired item (`--format jsonl` prints only these rows).
    pub items: bool,
    /// Do not record the comparison in the ledger.
    pub scratch: bool,
    /// Compare the arms as independent samples whose items need not overlap
    /// (Newcombe, Welch or a bootstrap per arm).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unpaired: bool,
    /// Also add the paired items one by one in a seeded order and report the
    /// confidence sequence, which stays valid however often it is inspected.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sequential: bool,
    /// Compare only the items of this part of the latest `evalsig split`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<Part>,
    /// Add ΔH, ΔF, Δd′ and Δc with a classification; the value is the fewest weighted
    /// items a cell needs (D-042).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdt: Option<f64>,
    pub dataset: DatasetChecks,
    pub arms: ArmChecks,
}

fn verdict_text(metric: &str, d: &compare::MetricDelta, level: f64) -> String {
    let pct = (level * 1000.0).round() / 10.0;
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
    let mut v = json!({
        "arms": { "baseline": baseline, "candidate": candidate },
        "primary_metric": primary,
        "n_used": c.n_used,
        "n_unpaired": c.n_unpaired,
        "n_input_changed": n_input_changed,
        "metrics": c.metrics,
        "transition": c.transition,
    });
    if let Some(s) = &c.sdt {
        v["sdt"] = json!(s);
    }
    v
}

/// Items present in both arms whose input differs: each arm's `input_ref` is the one its
/// run's own imports declare for the item (`Data::run_inputs`, D-035), else the item
/// record in force when the run's imports declared none.
fn changed_inputs(
    d: &Data,
    items: &BTreeMap<&str, &Item>,
    base: (&str, &Prepared<'_>),
    cand: (&str, &Prepared<'_>),
    overrides: &Overrides,
) -> (BTreeSet<String>, Vec<Finding>) {
    let inputs = |(run, p): (&str, &Prepared<'_>)| -> BTreeMap<String, Option<String>> {
        let declared = d.run_inputs.get(run);
        p.judged()
            .into_keys()
            .map(|k| {
                let input = declared
                    .and_then(|m| m.get(&k).cloned())
                    .or_else(|| items.get(k.as_str()).and_then(|i| i.input_ref.clone()));
                (k, input)
            })
            .collect()
    };
    let (a, b) = (inputs(base), inputs(cand));
    let a_ref: BTreeMap<&str, Option<&str>> =
        a.iter().map(|(k, v)| (k.as_str(), v.as_deref())).collect();
    let b_ref: BTreeMap<&str, Option<&str>> =
        b.iter().map(|(k, v)| (k.as_str(), v.as_deref())).collect();
    let (changed, finding) = validity::input_changed(&a_ref, &b_ref, overrides);
    let excluded = if overrides.allow_input_change {
        BTreeSet::new()
    } else {
        changed.iter().map(|s| (*s).to_owned()).collect()
    };
    (excluded, finding.into_iter().collect())
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
    let pair = [baseline, candidate];
    let sources: BTreeSet<String> = preferences
        .iter()
        .filter(|p| {
            p.outcome.is_some()
                && raters.contains(p.rater_id.as_str())
                && pair.contains(&p.left_run_id.as_str())
                && pair.contains(&p.right_run_id.as_str())
        })
        .filter_map(|p| json!(p.source).as_str().map(str::to_owned))
        .collect();
    Some(json!({
        "win_rate": win,
        "position_bias": bias,
        "raters": raters,
        "sources": sources,
    }))
}

/// How many recorded comparisons used this split, counting the one being made
/// (metrics spec §7.2). `entries` are the ledger lines before it.
pub fn validation_uses(entries: &[Entry], split: &str) -> u64 {
    1 + entries
        .iter()
        .filter(|e| {
            e.kind == Kind::Compare && e.summary.get("split").and_then(Value::as_str) == Some(split)
        })
        .count() as u64
}

/// Add `validation_uses` to the results and warn past the configured limit.
pub fn count_uses(results: &mut Value, warnings: &mut Vec<Warning>, entries: &[Entry], limit: u32) {
    // Dev comparisons are expected to repeat; only validation reuse is counted.
    if results["split_part"] == "dev" {
        return;
    }
    let Some(split) = results["split"].as_str() else {
        return;
    };
    let uses = validation_uses(entries, split);
    results["validation_uses"] = json!(uses);
    if uses > u64::from(limit) {
        warnings.push(Warning {
            code: "validation_uses".into(),
            message: format!(
                "this split has been compared on {uses} times (limit {limit}); time to re-split the validation data"
            ),
        });
    }
}

/// When the comparison could not tell the arms apart, how many items it would take:
/// a `power` command with the primary difference (or its minimum effect) and the
/// observed discordance of the decisions.
fn power_hint(results: &Value, verdict: Option<VerdictCode>) -> Option<Next> {
    if !matches!(
        verdict,
        Some(VerdictCode::Indistinguishable | VerdictCode::InsufficientData)
    ) {
        return None;
    }
    // The McNemar sizing of `power` is for accuracy, whose items all count.
    let primary = results["primary_metric"]
        .as_str()
        .filter(|m| *m == "accuracy")?;
    let m = results["metrics"]
        .as_array()?
        .iter()
        .find(|m| m["metric"] == primary)?;
    let delta = m["min_effect"]
        .as_f64()
        .filter(|e| *e > 0.0)
        .or_else(|| m["delta"].as_f64().map(f64::abs))
        .filter(|d| *d > 0.0)?;
    let t = &results["transition"];
    let moved: f64 = [
        "new_hits",
        "lost_hits",
        "new_false_alarms",
        "cleared_false_alarms",
    ]
    .iter()
    .filter_map(|k| t[*k].as_f64())
    .sum();
    let psi = moved / results["n_used"].as_f64().filter(|n| *n > 0.0)?;
    (psi > delta).then(|| Next {
        command: format!("evalsig power --delta {delta} --discordance {psi}"),
        why: "how many paired items would detect this difference".into(),
    })
}

/// What a comparison computes from data and configuration alone; `verify` repeats it.
pub struct Computed {
    pub results: Value,
    pub verdict: Option<EnvelopeVerdict>,
    pub findings: Vec<Finding>,
    pub warnings: Vec<Warning>,
    pub seed: u64,
}

/// `scope` is the split part that `--split` resolved to; items outside it are left out.
pub fn compute(
    d: &Data,
    config: &Config,
    args: &Request,
    scope: Option<&Scope>,
) -> Result<Computed, Error> {
    let items = d.items();
    let (labels, undated_skipped) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = labels.iter().collect();
    args.analysis.without_proxy("compare")?;
    let settings = analysis::settings(config, &args.analysis);
    let prepare = |id: &str| -> Result<(&Run, Prepared<'_>), Error> {
        let run = d.run(id)?;
        let p = report::prepare(run, &d.judgments(id), &items, &labels, &settings)
            .map_err(Error::report)?;
        Ok((run, p))
    };
    let (base_run, base) = prepare(&args.baseline)?;
    let baseline = Baseline::new(&base);
    let overrides = checks::overrides(args.dataset, args.arms);
    let mut findings = validity::dataset(&d.records, &overrides);
    let mut comparisons = Vec::new();
    let mut sequences = Vec::new();
    for cand_id in &args.candidates {
        let (cand_run, cand) = prepare(cand_id)?;
        if (cand.semantics(), cand.output_type(), cand.rule())
            != (base.semantics(), base.output_type(), base.rule())
        {
            return Err(Error::new(
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
        let (excluded, input_findings) = if args.unpaired {
            (BTreeSet::new(), Vec::new())
        } else {
            changed_inputs(
                d,
                &items,
                (&args.baseline, &base),
                (cand_id, &cand),
                &overrides,
            )
        };
        findings.extend(input_findings);
        let options = Options {
            level: settings.level,
            bootstrap: settings.bootstrap,
            seed: settings.seed,
            min_effect: config.compare.min_effect.clone(),
            excluded: excluded.clone(),
            scope: scope.map(|s| s.items.clone()),
            sdt_min_events: args.sdt,
        };
        if args.unpaired && args.sdt.is_some() {
            return Err(Error::usage_code(
                "conflicting_arguments",
                "--sdt needs paired arms; leave out --unpaired",
            ));
        }
        let c = if args.unpaired {
            compare::unpaired(&base, &cand, &options)
        } else {
            compare::compare(&baseline, &cand, &options)
        };
        if args.sdt.is_some() && c.sdt.is_none() {
            return Err(
                if base.positive().is_none() || base.semantics() != ValueSemantics::Decision {
                    Error::usage(
                        "compare --sdt needs decisions and a reference: a positive definition and a decision rule",
                    )
                } else {
                    Error::new(
                        "insufficient_data",
                        "the compared items have no positives or no negatives for --sdt",
                        Exit::InsufficientData,
                    )
                },
            );
        }
        let seq = args
            .sequential
            .then(|| compare::sequential(&baseline, &cand, &options));
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
        sequences.push(seq);
        comparisons.push((cand_id.clone(), cand, c, excluded.len()));
    }
    let all: Vec<Comparison> = comparisons.iter().map(|(_, _, c, _)| c.clone()).collect();
    let multi = compare::summarize(&all, args.metric.as_deref()).map_err(|available| {
        Error::usage_code(
            "unknown_metric",
            format!(
                "metric {:?} is not available for these runs",
                args.metric.as_deref().unwrap_or("")
            ),
        )
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
        .zip(&sequences)
        .map(|((id, cand, c, changed), seq)| {
            let mut v = comparison_json(&args.baseline, id, c, primary, *changed);
            if let Some(s) = seq {
                v["sequential"] = json!({ "order_seed": settings.seed, "metrics": s });
            }
            if args.unpaired {
                v["paired"] = Value::Bool(false);
            }
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
        "split": match scope {
            Some(s) => s.hash.clone(),
            None => item_set_hash(base.judged().keys())?,
        },
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
    if let Some(p) = args.split {
        results["split_part"] = json!(p);
    }
    let verdict = chosen.and_then(|i| {
        comparisons[i]
            .2
            .metrics
            .iter()
            .find(|m| m.metric == primary)
            .cloned()
    });
    // Runs repeated for one system version give the run-to-run spread of the primary
    // metric: a diagnostic beside the interval, never part of it (metrics spec §6.3).
    let spread_delta = verdict.as_ref().and_then(|m| m.delta);
    if let (Some(delta), Some(i), true) = (
        spread_delta,
        chosen,
        config.checks.warn_run_variance_exceeds_delta,
    ) {
        let mut runs: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
        for r in &d.records {
            if let Record::Run(run) = r {
                let key = (run.system.id.as_str(), run.system.version.as_str());
                runs.entry(key).or_default().push(&run.run_id);
            }
        }
        let prepared: BTreeMap<&str, &Prepared<'_>> =
            std::iter::once((args.baseline.as_str(), &base))
                .chain(comparisons.iter().map(|(id, p, _, _)| (id.as_str(), p)))
                .collect();
        let cand = &comparisons[i].1;
        // Every run is measured on the items the difference was measured on.
        let items: Option<BTreeSet<String>> = base
            .metric_ids()
            .into_iter()
            .find(|id| id.name() == primary)
            .map(|id| {
                let b = base.item_values(id, Fill::Policy);
                cand.item_values(id, Fill::Policy)
                    .into_keys()
                    .filter(|k| b.contains_key(k))
                    .filter(|k| scope.is_none_or(|s| s.items.contains(k)))
                    .collect()
            });
        let point = |p: &Prepared<'_>| {
            let id = p.metric_ids().into_iter().find(|id| id.name() == primary)?;
            let mut v = p.item_values(id, Fill::Policy);
            if let Some(items) = &items {
                v.retain(|k, _| items.contains(k));
            }
            compare::total_ratio(&v)
        };
        let versions: BTreeSet<(&str, &str)> = [base_run, d.run(&comparisons[i].0)?]
            .into_iter()
            .map(|r| (r.system.id.as_str(), r.system.version.as_str()))
            .collect();
        let mut spread: Option<f64> = None;
        for version in versions {
            let ids = &runs[&version];
            if ids.len() < 2 {
                continue;
            }
            let mut values = Vec::new();
            for &id in ids {
                // A sibling that cannot be prepared for this task is not a repeat.
                let value = match prepared.get(id) {
                    Some(p) => point(p),
                    None => prepare(id).ok().and_then(|(_, p)| point(&p)),
                };
                values.extend(value);
            }
            if let Some(sd) = replicates::run_to_run_sd(&values) {
                spread = Some(spread.map_or(sd, |s| s.max(sd)));
            }
        }
        if let Some(sd) = spread {
            findings.extend(validity::run_variance_exceeds_delta(sd, delta, &overrides));
        }
    }
    let mut warnings = Vec::new();
    if comparisons.len() > 1 {
        warnings.push(Warning {
            code: "selected_after_comparison".into(),
            message:
                "the best of several candidates is biased upwards; confirm it on another split"
                    .into(),
        });
    }
    let ec = comparisons
        .iter()
        .flat_map(|(_, _, c, _)| &c.metrics)
        .find(|m| m.id == MetricId::ExpectedCost);
    // Unpaired comparisons do not compare expected cost (D-041).
    if !args.unpaired {
        warnings.extend(analysis::cost_warnings(
            config,
            base.semantics(),
            ec.is_some(),
            ec.and_then(|m| m.by_stratum.as_ref())
                .map(|b| (b.present(), b.mix)),
        ));
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

pub fn run(root: &Path, args: &Request) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let scope = args
        .split
        .map(|p| split::scope(&project, p, None))
        .transpose()?;
    let mut c = compute(&d, &config, args, scope.as_ref())?;
    let entries = project.entries()?;
    count_uses(
        &mut c.results,
        &mut c.warnings,
        &entries,
        config.compare.validation_warn_after,
    );
    let next = power_hint(&c.results, c.verdict.as_ref().map(|v| v.code));
    let mut envelope = Envelope::new(Schema::Compare, c.results);
    envelope.verdict = c.verdict;
    envelope.next.extend(next);
    envelope.warnings = c.warnings;
    let request = provenance::Request::Compare(args.clone());
    if args.scratch {
        provenance::fill(&project, &config, &d, &mut envelope, c.seed, &request)?;
    } else {
        let summary = json!({
            "baseline": args.baseline,
            "candidates": args.candidates,
            "metric": envelope.results["primary_metric"],
            "split": envelope.results["split"],
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
