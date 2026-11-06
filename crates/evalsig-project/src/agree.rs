//! `evalsig agree`: agreement between raters, the variability of one rater, grader and
//! system agreement with people under the same rule, and agreement on pairwise
//! preferences (SPEC §10.1, metrics spec §9.1, §9.5, §9.7, §10).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use evalsig_core::agreement;
use evalsig_core::checks::{Finding, Overrides};
use evalsig_core::meta::{self, MetaEvaluation};
use evalsig_core::parity::{self, ParityVerdict};
use evalsig_core::preference::{self, RankingError};
use evalsig_core::records::{Label, LabelKind, Preference, Record, Source, Value as RecordValue};
use evalsig_core::reference;
use evalsig_core::report;
use evalsig_core::rules::Rule;
use serde_json::{Value, json};

use crate::Error;
use crate::analysis::{self, Analysis, ReferenceChoice};
use crate::checks;
use crate::data;
use crate::envelope::{
    Envelope, Output, Verdict as EnvelopeVerdict, VerdictCode, Warning, round_numbers,
};
use crate::exit::Exit;
use crate::project::Project;

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub analysis: Analysis,
    /// Also compare this run's decisions with each human rater, under the same rule
    /// as the raters among themselves.
    pub run: Option<String>,
    /// Agreement of raters on pairwise preferences, their position bias, and the
    /// Bradley-Terry ranking of the arms.
    pub preference: bool,
    /// The alternative annotator test: could the --run system replace one of the human
    /// raters (D-017)?
    pub parity: bool,
}

/// Each rater's value per item: the latest revision of their first label replicate;
/// adjudications are not independent and are left out (metrics spec §9.5).
type Ratings<'a> = BTreeMap<&'a str, BTreeMap<&'a str, f64>>;

fn ratings<'a>(labels: &[&'a Label], value: impl Fn(&Label) -> Option<f64>) -> Ratings<'a> {
    let mut out: Ratings<'a> = BTreeMap::new();
    for l in labels {
        if l.kind() == LabelKind::Adjudication || l.label_replicate() != 0 {
            continue;
        }
        if let Some(v) = value(l) {
            out.entry(l.rater_id.as_str())
                .or_default()
                .insert(l.item_id.as_str(), v);
        }
    }
    out
}

fn shared(a: &BTreeMap<&str, f64>, b: &BTreeMap<&str, f64>) -> Vec<(f64, f64)> {
    a.iter()
        .filter_map(|(i, x)| b.get(i).map(|y| (*x, *y)))
        .collect()
}

/// Ordered when every value is an integer and there are more than two of them.
fn ordered(r: &Ratings<'_>) -> bool {
    let values: BTreeSet<u64> = r
        .values()
        .flat_map(|m| m.values().map(|v| v.to_bits()))
        .collect();
    values.len() > 2
        && r.values()
            .flat_map(|m| m.values())
            .all(|v| v.fract() == 0.0)
}

/// The agreement measure the same-rule table averages.
fn headline(m: &MetaEvaluation) -> Option<f64> {
    m.cohen_kappa.or(m.quadratic_weighted_kappa)
}

pub fn run(root: &Path, args: &Request) -> Result<Output, Error> {
    if matches!(args.analysis.reference, Some(ReferenceChoice::Latent)) {
        return Err(Error::new(
            "report",
            "the latent reference mode is not available in this version",
            Exit::Rejected,
        ));
    }
    args.analysis.without_proxy("agree")?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let task = args.analysis.task.as_str();
    let (all, _) = d.labels(args.analysis.labels_as_of.as_deref())?;
    let labels: Vec<&Label> = all.iter().filter(|l| l.task() == task).collect();
    let source: BTreeMap<&str, Source> = labels
        .iter()
        .map(|l| (l.rater_id.as_str(), l.source))
        .collect();
    let positive: Option<Rule> =
        report::positive_rule(config.positive().for_task(task).copied(), &labels);
    let raw = ratings(&labels, |l| l.value.map(RecordValue::as_f64));
    let decided = positive.map(|p| {
        ratings(&labels, |l| {
            l.value
                .and_then(|v| p.is_positive(v))
                .map(|b| f64::from(u8::from(b)))
        })
    });
    let is_ordered = ordered(&raw);
    let evaluate = |a: &str, b: &str| -> (usize, MetaEvaluation, Option<MetaEvaluation>) {
        let values = shared(&raw[a], &raw[b]);
        // A rater whose values the positive rule cannot read has no decisions.
        let decisions = decided.as_ref().and_then(|r| {
            let (x, y) = (r.get(a)?, r.get(b)?);
            Some(meta::evaluate(&shared(x, y), false))
        });
        (values.len(), meta::evaluate(&values, is_ordered), decisions)
    };
    let raters: Vec<&str> = raw.keys().copied().collect();
    let humans: Vec<&str> = raters
        .iter()
        .copied()
        .filter(|r| source.get(r) == Some(&Source::Human))
        .collect();
    let mut pairs = Vec::new();
    let mut kappa: BTreeMap<(&str, &str), f64> = BTreeMap::new();
    for (i, a) in raters.iter().enumerate() {
        for b in &raters[i + 1..] {
            let (n, values, decisions) = evaluate(a, b);
            if n == 0 {
                continue;
            }
            if let Some(k) = decisions
                .as_ref()
                .map_or_else(|| headline(&values), headline)
            {
                kappa.insert((a, b), k);
                kappa.insert((b, a), k);
            }
            pairs.push(json!({
                "raters": [a, b],
                "sources": [source.get(a), source.get(b)],
                "items": n,
                "values": values,
                "decisions": decisions,
            }));
        }
    }
    // The same rule for everyone: each one against each human but themselves.
    let mean_against = |who: &str| -> (Option<f64>, usize) {
        let ks: Vec<f64> = humans
            .iter()
            .filter(|h| **h != who)
            .filter_map(|h| kappa.get(&(who, *h)).copied())
            .collect();
        (evalsig_core::numeric::mean(ks.iter().copied()), ks.len())
    };
    let mut same_rule: Vec<Value> = raters
        .iter()
        .map(|r| {
            let (k, n) = mean_against(r);
            json!({"who": r, "kind": source.get(r), "mean_kappa": k, "humans_compared": n})
        })
        .collect();
    let mut parity_block = None;
    if let Some(run_id) = &args.run {
        if args.parity {
            parity_block = Some(parity(&d, &config, &args.analysis, &labels, run_id)?);
        }
        let votes: BTreeMap<&str, BTreeMap<&str, bool>> = decided
            .as_ref()
            .map(|r| {
                humans
                    .iter()
                    .filter_map(|h| {
                        r.get(h)
                            .map(|m| (*h, m.iter().map(|(i, v)| (*i, *v == 1.0)).collect()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        same_rule.push(system_row(
            &d,
            &config,
            &args.analysis,
            &labels,
            run_id,
            &votes,
        )?);
    }
    let shares = positive
        .as_ref()
        .map_or_else(Vec::new, |p| soft_shares(&labels, p));
    let variability: BTreeMap<String, Value> = agreement::variability(labels.iter().copied())
        .into_iter()
        .map(|(t, (within, between))| {
            (
                t,
                json!({
                    "within_rater": within, "within_rate": within.rate(),
                    "between_raters": between, "between_rate": between.rate(),
                }),
            )
        })
        .collect();
    let mut results = json!({
        "task": task,
        "raters": raters.iter().map(|r| json!({"rater": r, "source": source.get(r), "items": raw[r].len()})).collect::<Vec<_>>(),
        "ordered_values": is_ordered,
        "pairs": pairs,
        "same_rule": same_rule,
        "variability": variability,
        "ceiling": reference::accuracy_ceiling(&shares),
    });
    let mut findings: Vec<Finding> = Vec::new();
    if args.preference {
        let prefs: Vec<&Preference> = d
            .records
            .iter()
            .filter_map(|r| match r {
                Record::Preference(p) if p.task() == task => Some(p),
                _ => None,
            })
            .collect();
        results["preference"] = preference_block(&prefs, &config, &mut findings)?;
    }
    let parity = parity_block.map(|p| {
        let (wins, n) = (p.tested.iter().filter(|t| t.system_wins).count(), p.tested.len());
        let (code, text) = match p.verdict {
            ParityVerdict::HumanParity => (
                VerdictCode::HumanParity,
                format!(
                    "the system agrees with people as well as people do: it wins against {wins} of {n} human raters (epsilon {})",
                    p.epsilon
                ),
            ),
            ParityVerdict::BelowHuman => (
                VerdictCode::BelowHuman,
                format!(
                    "the system wins against {wins} of {n} human raters, fewer than half (epsilon {})",
                    p.epsilon
                ),
            ),
            ParityVerdict::InsufficientData => (
                VerdictCode::InsufficientData,
                format!(
                    "fewer than two human raters share {} items with the others",
                    parity::MIN_ITEMS
                ),
            ),
        };
        results["parity"] = json!(p);
        (EnvelopeVerdict { code, text }, n == 2)
    });
    let mut envelope = Envelope::new("evalsig.agree/1", round_numbers(results));
    if let Some((verdict, two_raters)) = parity {
        envelope.verdict = Some(verdict);
        if two_raters {
            envelope.warnings.push(Warning {
                code: "two_human_raters".into(),
                message: "only two human raters were tested; parity is less reliable than with three or more".into(),
            });
        }
    }
    envelope.provenance.snapshots = d.snapshot_ids();
    Ok(checks::apply(envelope, &findings))
}

/// The same-rule row of a run: Cohen's κ of its decisions against each human rater's
/// decisions, averaged.
fn system_row(
    d: &data::Data,
    config: &evalsig_core::config::Config,
    analysis_args: &Analysis,
    labels: &[&Label],
    run_id: &str,
    human_votes: &BTreeMap<&str, BTreeMap<&str, bool>>,
) -> Result<Value, Error> {
    let settings = analysis::settings(config, analysis_args);
    let prepared = report::prepare(
        d.run(run_id)?,
        &d.judgments(run_id),
        &d.items(),
        labels,
        &settings,
    )
    .map_err(Error::report)?;
    let judged = prepared.judged();
    // The run's decision on an item: positive when most of its replicates are.
    let system: BTreeMap<&str, bool> = judged
        .iter()
        .map(|(item, (v, _))| (item.as_str(), *v >= 0.5))
        .collect();
    let (mean, each) = reference::kappa_vs_raters(&system, human_votes).unzip();
    Ok(json!({
        "who": run_id,
        "kind": "system",
        "mean_kappa": mean,
        "humans_compared": each.map_or(0, |e| e.len()),
    }))
}

/// The alternative annotator test of a run against the human raters (metrics spec §9.4).
fn parity(
    d: &data::Data,
    config: &evalsig_core::config::Config,
    analysis_args: &Analysis,
    labels: &[&Label],
    run_id: &str,
) -> Result<parity::Parity, Error> {
    let settings = analysis::settings(config, analysis_args);
    let prepared = report::prepare(
        d.run(run_id)?,
        &d.judgments(run_id),
        &d.items(),
        labels,
        &settings,
    )
    .map_err(Error::report)?;
    Ok(parity::of_run(
        &prepared,
        labels,
        config.agree.parity_epsilon,
        parity::Q,
    ))
}

/// Each item's soft reference from its human raters, for the ceiling, which is
/// defined against the soft reference whatever `--reference` says (metrics spec §9.1).
fn soft_shares(labels: &[&Label], positive: &Rule) -> Vec<f64> {
    let mut by_item: BTreeMap<&str, Vec<&Label>> = BTreeMap::new();
    for l in labels.iter().filter(|l| l.source == Source::Human) {
        by_item.entry(l.item_id.as_str()).or_default().push(l);
    }
    by_item
        .values()
        .filter_map(|ls| {
            reference::of(ls, positive, evalsig_core::config::ReferenceMode::Soft).map(|r| r.share)
        })
        .collect()
}

/// Rater agreement on normalised preference verdicts and each rater's position bias,
/// for every compared pair of runs, and the ranking of all arms.
fn preference_block(
    prefs: &[&Preference],
    config: &evalsig_core::config::Config,
    findings: &mut Vec<Finding>,
) -> Result<Value, Error> {
    let level = config.stats.level;
    let arms: BTreeSet<(&str, &str)> = prefs
        .iter()
        .map(|p| {
            let (l, r) = (p.left_run_id.as_str(), p.right_run_id.as_str());
            if l <= r { (l, r) } else { (r, l) }
        })
        .collect();
    let comparisons: Vec<Value> = arms
        .iter()
        .map(|(a, b)| {
            let judged = preference::normalise(prefs.iter().copied(), a, b);
            let raters: BTreeSet<&str> = judged.iter().map(|j| j.rater_id).collect();
            let by_rater: BTreeMap<&str, Value> = raters
                .iter()
                .map(|r| {
                    let mine: Vec<_> = judged
                        .iter()
                        .filter(|j| j.rater_id == *r)
                        .cloned()
                        .collect();
                    (*r, json!(preference::position_bias(&mine, level)))
                })
                .collect();
            json!({
                "arms": [a, b],
                "rater_agreement": preference::rater_agreement(&judged),
                "position_bias_by_rater": by_rater,
            })
        })
        .collect();
    let ranking = match preference::ranking(
        prefs.iter().copied(),
        level,
        config.stats.bootstrap,
        config.stats.seed.unwrap_or(0),
    ) {
        Ok(r) => json!(r),
        Err(RankingError::Disconnected) => {
            findings.push(Finding::new(
                "comparison_graph_disconnected",
                "the compared arms form more than one group, so no ranking is given",
                &Overrides::default(),
            ));
            Value::Null
        }
        Err(e) => json!({ "unavailable": e.to_string() }),
    };
    Ok(json!({ "comparisons": comparisons, "ranking": ranking }))
}
