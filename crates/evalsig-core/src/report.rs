//! The default-tier report of one run (SPEC §6, metrics spec §0–§2, §6, §8, §9.2).
//!
//! Missing replicates are filled or dropped per metric under the missing policy,
//! replicates combine into one value per item under the estimand, and items (or their
//! clusters) are the resampling unit for intervals.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bootstrap::{self, Strata};
use crate::config::{Estimand, RaterDesign, ReferenceMode};
use crate::interval::{clopper_pearson, wilson_two_sided};
use crate::missing::{MissingPolicy, Policy, Side};
use crate::numeric::{Rng, Running, mean, sum};
use crate::records::{
    Item, Judgment, Label, MissingReason, OutputType, Run, Value, ValueSemantics,
};
use crate::reference;
use crate::replicates::{Disagreement, decided_positive, item_value};
use crate::rules::Rule;

mod detail;
pub use detail::{Calibration, FeatureRow, ProxyEstimate, Roc, Sdt};

/// What a report needs besides the records.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Task whose judgments are reported.
    pub task: String,
    /// Positive definition for reference labels.
    pub positive: Option<Rule>,
    /// Decision rule for judgments.
    pub decision: Option<Rule>,
    /// For score runs: report the share at or above this threshold.
    pub threshold: Option<f64>,
    /// Which performance is estimated.
    pub estimand: Estimand,
    /// Missing policy applied per reason.
    pub missing: MissingPolicy,
    /// Reference mode for items with several human raters.
    pub reference: ReferenceMode,
    /// Rater design of the labels.
    pub rater_design: RaterDesign,
    /// Confidence level of the intervals.
    pub level: f64,
    /// Number of bootstrap resamples.
    pub bootstrap: u32,
    /// Seed for resampling.
    pub seed: u64,
    /// Production prevalence for the converted precision.
    pub prevalence: Option<f64>,
    /// Cost matrices and prevalences for expected cost (metrics spec §4, D-041).
    pub cost: Option<crate::config::Cost>,
    /// What the report includes beside its metrics.
    pub sections: Sections,
    /// Bound rates with the exact (Clopper-Pearson) interval on the effective number
    /// of items, as pass/fail decisions need (`gate`, `check`; metrics spec §1).
    pub exact_rates: bool,
    /// Correct the human-scale value with this model rater's labels (PPI++,
    /// `report --reference human --proxy`).
    pub proxy: Option<String>,
}

/// What a report includes beside its metrics. None of it changes a metric's value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    /// Name of the stratum key to report separately.
    pub by: Option<String>,
    /// Include one row per item.
    pub items: bool,
    /// Add the detail tier's ROC and calibration (`report`; other commands that only
    /// read the metrics leave it off).
    pub detail: bool,
    /// Add the feature likelihood ratios (`report --features`).
    pub features: bool,
}

/// Why a report could not be built.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReportError {
    /// The run has no judgments for the task (run, task).
    #[error("run {0:?} has no judgments for task {1:?}")]
    NoJudgments(String, String),
    /// The named reference mode cannot be used.
    #[error("the {0} reference mode is not available in this version")]
    UnsupportedReference(&'static str),
    /// Ordinal and score decisions have no decision rule.
    #[error("ordinal and score decisions need a decision rule (`decision` or --threshold)")]
    NoDecisionRule,
    /// Labels of the named kind need a positive definition.
    #[error("{0} labels need a positive definition (`positive`)")]
    NoPositiveDefinition(&'static str),
    /// The named estimand needs pass or fail judgments.
    #[error("the {0} estimand needs pass or fail judgments; score runs need --threshold")]
    EstimandNeedsBinary(String),
    /// A threshold was given for binary judgments.
    #[error("--threshold applies to score outputs; this run's judgments are binary")]
    ThresholdOnBinary,
    /// The named rater has no item that also has a judgment.
    #[error("no item has both a label by {0:?} and a judgment, so it cannot serve as the proxy")]
    NoProxyLabels(String),
}

/// One interval-bearing metric.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Metric {
    /// Metric name.
    pub metric: &'static str,
    /// Estimate; absent when its denominator is empty.
    pub value: Option<f64>,
    /// Interval at the report level; absent when no interval can be computed.
    pub ci: Option<(f64, f64)>,
    /// Interval method used.
    pub interval: &'static str,
    /// Bootstrap resamples used, when the interval is a bootstrap one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resamples: Option<u32>,
    /// System-side missing values filled with the worst and the best judgment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_bounds: Option<(f64, f64)>,
    /// In a crossed design: the lowest and highest value with each rater alone as the
    /// reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rater_spread: Option<(f64, f64)>,
    /// Number of items the value is computed from (weighted).
    pub n: f64,
}

/// Replicate disagreement of one kind of replicate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReproducibilityRow {
    /// Mean share of differing replicate pairs per item.
    pub pairwise_disagreement: f64,
    /// Share of items with any differing replicate pair.
    pub items_with_any_disagreement: f64,
    /// Items with at least two replicates.
    pub items: u64,
    /// One-sided upper bound on the pairwise disagreement at the report level.
    pub upper_bound: f64,
}

/// How the reference was built.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReferenceInfo {
    /// Reference mode used.
    pub mode: ReferenceMode,
    /// Number of raters.
    pub raters: usize,
    /// Items that have a reference.
    pub items_with_reference: usize,
    /// Items whose reference comes from a model rater.
    pub model_derived_items: usize,
    /// Items weighted by 1 / label_inclusion_prob.
    #[serde(skip_serializing_if = "is_zero")]
    pub inclusion_weighted_items: usize,
    /// Rater design details, flattened into the object.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub design: Option<reference::DesignInfo>,
    /// In a crossed design: each metric with each rater alone as the reference.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub by_rater: BTreeMap<String, BTreeMap<&'static str, f64>>,
}

/// One item of a per-item report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemRow {
    /// Item ID.
    pub item_id: String,
    /// Reference share of raters calling the item positive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<f64>,
    /// Combined judged value under the estimand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judged: Option<f64>,
    /// Number of generation replicates.
    pub replicates: usize,
    /// Missing reasons of the item's missing replicates.
    pub missing: Vec<MissingReason>,
}

/// The default-tier report of one run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Run ID.
    pub run: String,
    /// Task name.
    pub task: String,
    /// Value semantics of the run.
    pub value_semantics: ValueSemantics,
    /// Output type of the judgments.
    pub output_type: OutputType,
    /// Positive definition used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positive: Option<Rule>,
    /// Decision rule used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<Rule>,
    /// Estimand, in its string form.
    pub estimand: String,
    /// Reference details; absent without a positive definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceInfo>,
    /// Number of items.
    pub n_items: usize,
    /// Number of judgments.
    pub n_judgments: usize,
    /// Number of missing judgments, every reason counted.
    pub n_missing: u64,
    /// Missing judgments per reason.
    pub missing_by_reason: BTreeMap<MissingReason, u64>,
    /// Missing policy applied per reason.
    pub missing_policy: BTreeMap<MissingReason, Policy>,
    /// Items with fewer replicates than the estimand's k.
    pub n_insufficient_replicates: usize,
    /// Confidence level of the intervals.
    pub level: f64,
    /// Seed used for resampling.
    pub seed: u64,
    /// Metrics with intervals.
    pub metrics: Vec<Metric>,
    /// Replicate disagreement keyed by `generation` or `grading`.
    pub reproducibility: BTreeMap<&'static str, ReproducibilityRow>,
    /// Metrics per stratum.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub by: BTreeMap<String, Vec<Metric>>,
    /// Per-item rows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemRow>,
    /// ROC and AUC of the detail tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roc: Option<Roc>,
    /// d′ and c of the detail tier, for runs with a decision rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdt: Option<Sdt>,
    /// Calibration of the detail tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration: Option<Calibration>,
    /// Feature likelihood ratios.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<FeatureRow>>,
    /// Proxy-corrected human-scale estimate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyEstimate>,
    /// Expected cost under `[cost]` (D-041).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_cost: Option<crate::cost::CostSummary>,
}

/// One item of the run: its replicates (judged value or missing reason, one per
/// generation replicate, grading replicates averaged), reference share and weight.
#[derive(Debug, Clone)]
struct Unit {
    item_id: String,
    cluster: String,
    weight: f64,
    stratum: Option<String>,
    /// Generation replicates: the value (judged positive share, or score), or the
    /// reason it is missing.
    replicates: Vec<Result<f64, MissingReason>>,
    /// Grading replicates of each generation replicate, for grader reproducibility.
    grades: Vec<Vec<f64>>,
    /// For score runs with a threshold: per generation replicate, the share of its
    /// grading replicates at or above the threshold.
    at_least: Vec<Result<f64, MissingReason>>,
    /// Share of raters calling the item positive.
    reference: Option<f64>,
    model_reference: bool,
    /// Each rater's vote (1 positive, 0 negative).
    votes: BTreeMap<String, f64>,
    /// Whether the weight includes 1 / label_inclusion_prob.
    inclusion_weighted: bool,
    /// The mean raw value of the judgments with a value (scores, levels).
    raw: Option<f64>,
    /// The raw values of the judgments with a value, for expected cost.
    levels: Vec<f64>,
    /// The item's value of the `[cost] by` stratum.
    cost_stratum: Option<String>,
    /// Each feature's share of the item's judgments that fired it.
    fired: BTreeMap<String, f64>,
}

fn judged_value(j: &Judgment, rule: Option<&Rule>, semantics: ValueSemantics) -> Option<f64> {
    let v = j.value?;
    match (semantics, rule) {
        (ValueSemantics::Score, None) => Some(v.as_f64()),
        (_, Some(r)) => r.is_positive(v).map(|p| f64::from(u8::from(p))),
        (ValueSemantics::Decision, None) => match v {
            Value::Bool(b) => Some(f64::from(u8::from(b))),
            _ => None,
        },
    }
}

/// One run's items ready for metrics: replicates grouped per item, references built,
/// and the rule that turns judgments into decisions.
pub struct Prepared<'a> {
    settings: &'a Settings,
    run_id: String,
    semantics: ValueSemantics,
    output_type: OutputType,
    rule: Option<Rule>,
    positive: Option<Rule>,
    units: Vec<Unit>,
    missing_by_reason: BTreeMap<MissingReason, u64>,
    raters: usize,
    n_judgments: usize,
    at_least: Option<f64>,
    score_range: (f64, f64),
}

/// Group a run's judgments into items and build their references.
pub fn prepare<'a>(
    run: &Run,
    judgments: &[&Judgment],
    items: &BTreeMap<&str, &Item>,
    labels: &[&Label],
    s: &'a Settings,
) -> Result<Prepared<'a>, ReportError> {
    if s.reference == ReferenceMode::Latent {
        return Err(ReportError::UnsupportedReference("latent"));
    }
    let semantics = run.value_semantics();
    let judgments: Vec<&Judgment> = judgments
        .iter()
        .copied()
        .filter(|j| j.task() == s.task)
        .collect();
    let output_type = run
        .output_type
        .or_else(|| judgments.first().map(|j| j.output_type))
        .ok_or_else(|| ReportError::NoJudgments(run.run_id.clone(), s.task.clone()))?;
    if judgments.is_empty() {
        return Err(ReportError::NoJudgments(run.run_id.clone(), s.task.clone()));
    }
    // The configured decision rule (or --threshold) says where a system decision turns
    // positive. A score run keeps its scores; a threshold there adds the share of
    // replicates at or above it.
    let decision = match (semantics, s.threshold) {
        (ValueSemantics::Decision, Some(t)) => Some(Rule::ScoreAtLeast(t)),
        (ValueSemantics::Decision, None) => s.decision,
        (ValueSemantics::Score, _) => None,
    };
    let at_least = s
        .threshold
        .filter(|_| semantics == ValueSemantics::Score && output_type != OutputType::Binary);
    // A decision run needs a rule unless its output is binary; a score run uses its
    // scores directly unless a threshold turns them into pass or fail.
    if s.threshold.is_some() && output_type == OutputType::Binary {
        return Err(ReportError::ThresholdOnBinary);
    }
    // A project-wide decision rule written for ordinal or score runs does not read
    // booleans; binary runs then use their own value.
    let decision = decision
        .filter(|r| output_type != OutputType::Binary || matches!(r, Rule::Equals(Value::Bool(_))));
    let rule = match (semantics, output_type, decision) {
        (_, _, Some(r)) => Some(r),
        (ValueSemantics::Decision, OutputType::Binary, None) => {
            Some(Rule::Equals(Value::Bool(true)))
        }
        (ValueSemantics::Decision, _, None) => return Err(ReportError::NoDecisionRule),
        (ValueSemantics::Score, OutputType::Binary, None) => Some(Rule::Equals(Value::Bool(true))),
        (ValueSemantics::Score, _, None) => None,
    };
    if s.estimand != Estimand::SingleShot && rule.is_none() && at_least.is_none() {
        return Err(ReportError::EstimandNeedsBinary(s.estimand.to_string()));
    }

    let positive = if semantics == ValueSemantics::Decision {
        Some(
            positive_rule(s.positive, labels)
                .ok_or(ReportError::NoPositiveDefinition("ordinal and score"))?,
        )
    } else {
        None
    };

    // Group judgments into units, ordered by item ID.
    let mut by_item: BTreeMap<&str, BTreeMap<u32, Vec<&Judgment>>> = BTreeMap::new();
    for j in &judgments {
        by_item
            .entry(j.item_id.as_str())
            .or_default()
            .entry(j.gen_replicate)
            .or_default()
            .push(j);
    }
    let mut labels_by_item: BTreeMap<&str, Vec<&Label>> = BTreeMap::new();
    for l in labels.iter().filter(|l| l.task() == s.task) {
        labels_by_item
            .entry(l.item_id.as_str())
            .or_default()
            .push(l);
    }
    let mut missing_by_reason: BTreeMap<MissingReason, u64> = BTreeMap::new();
    let mut raters = BTreeSet::new();
    let mut units = Vec::new();
    for (item_id, gens) in &by_item {
        let item = items.get(item_id);
        let mut replicates = Vec::new();
        let mut at_least_reps = Vec::new();
        let mut grades = Vec::new();
        let mut raw = Vec::new();
        let mut fired: BTreeMap<String, f64> = BTreeMap::new();
        let mut n_judgments = 0.0;
        for reps in gens.values() {
            let mut values = Vec::new();
            let mut reason = None;
            for j in reps {
                if j.missing.is_none()
                    && let Some(v) = j.value
                {
                    raw.push(v.as_f64());
                }
                n_judgments += 1.0;
                for f in j.features.iter().flatten() {
                    *fired.entry(f.feature_id.clone()).or_default() += f64::from(u8::from(f.fired));
                }
                match (&j.missing, judged_value(j, rule.as_ref(), semantics)) {
                    (Some(m), _) => {
                        *missing_by_reason.entry(m.reason).or_default() += 1;
                        reason.get_or_insert(m.reason);
                    }
                    (None, Some(v)) => values.push(v),
                    (None, None) => {
                        *missing_by_reason
                            .entry(MissingReason::InvalidOutput)
                            .or_default() += 1;
                        reason.get_or_insert(MissingReason::InvalidOutput);
                    }
                }
            }
            match (values.is_empty(), reason) {
                (false, _) => {
                    replicates.push(Ok(
                        mean(values.iter().copied()).expect("values is not empty")
                    ));
                    if let Some(t) = at_least {
                        let hits = values.iter().filter(|v| **v >= t).count() as f64;
                        at_least_reps.push(Ok(hits / values.len() as f64));
                    }
                }
                (true, Some(r)) => {
                    replicates.push(Err(r));
                    at_least_reps.push(Err(r));
                }
                (true, None) => {}
            }
            grades.push(values);
        }
        let item_labels = labels_by_item.get(item_id).cloned().unwrap_or_default();
        for l in &item_labels {
            raters.insert(l.rater_id.as_str());
        }
        let reference = positive
            .as_ref()
            .and_then(|p| reference::of(&item_labels, p, s.reference));
        let votes = positive.as_ref().map_or_else(BTreeMap::new, |p| {
            reference::votes(&item_labels, p)
                .into_iter()
                .map(|(r, v)| (r.to_owned(), f64::from(u8::from(v))))
                .collect()
        });
        // A stratum value as text: a string as it is, anything else as JSON.
        let stratum_of = |name: &str| {
            item.and_then(|i| i.strata.as_ref())
                .and_then(|m| m.get(name))
                .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
        };
        // Inverse-probability weights correct partial verification (metrics spec §9.9).
        let inverse_inclusion = reference
            .and(item.and_then(|i| i.label_inclusion_prob))
            .map(|p| 1.0 / p);
        units.push(Unit {
            item_id: (*item_id).to_owned(),
            cluster: item.map_or((*item_id).to_owned(), |i| i.cluster_key().to_owned()),
            weight: item.map_or(1.0, |i| i.weight()) * inverse_inclusion.unwrap_or(1.0),
            inclusion_weighted: inverse_inclusion.is_some(),
            raw: mean(raw.iter().copied()),
            cost_stratum: s
                .cost
                .as_ref()
                .and_then(|c| c.by.as_deref())
                .and_then(stratum_of),
            levels: if s.cost.is_some() { raw } else { Vec::new() },
            // A judgment that does not report a feature counts as not firing it.
            fired: fired
                .into_iter()
                .map(|(k, f)| (k, f / n_judgments))
                .collect(),
            stratum: s.sections.by.as_ref().and_then(|name| {
                if name == reference::AGREEMENT_STRATUM {
                    return positive
                        .as_ref()
                        .and_then(|p| reference::agreement_stratum(&item_labels, p))
                        .map(str::to_owned);
                }
                stratum_of(name)
            }),
            replicates,
            grades,
            at_least: at_least_reps,
            reference: reference.map(|r| r.share),
            model_reference: reference.is_some_and(|r| r.model_derived),
            votes,
        });
    }

    let scores: Vec<f64> = units
        .iter()
        .flat_map(|u| u.replicates.iter().filter_map(|r| r.as_ref().ok().copied()))
        .collect();
    let (score_min, score_max) = scores
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &x| {
            (a.min(x), b.max(x))
        });
    Ok(Prepared {
        settings: s,
        run_id: run.run_id.clone(),
        semantics,
        output_type,
        rule,
        positive,
        units,
        missing_by_reason,
        raters: raters.len(),
        n_judgments: judgments.len(),
        at_least,
        score_range: (
            if score_min.is_finite() {
                score_min
            } else {
                0.0
            },
            if score_max.is_finite() {
                score_max
            } else {
                1.0
            },
        ),
    })
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The configured positive definition, or `true` when the labels are booleans.
pub fn positive_rule(configured: Option<Rule>, labels: &[&Label]) -> Option<Rule> {
    configured.or_else(|| {
        labels
            .iter()
            .any(|l| matches!(l.value, Some(Value::Bool(_))))
            .then_some(Rule::Equals(Value::Bool(true)))
    })
}

/// Build the report of one run from its records.
pub fn build(
    run: &Run,
    judgments: &[&Judgment],
    items: &BTreeMap<&str, &Item>,
    labels: &[&Label],
    s: &Settings,
) -> Result<Report, ReportError> {
    let p = prepare(run, judgments, items, labels, s)?;
    from_prepared(&p, items, labels)
}

/// The report of units already prepared (with `items` and `labels` they came from,
/// which the proxy correction reads).
pub fn from_prepared(
    p: &Prepared<'_>,
    items: &BTreeMap<&str, &Item>,
    labels: &[&Label],
) -> Result<Report, ReportError> {
    let s = p.settings;
    let calc = p.calc();
    let units = &p.units;
    let semantics = p.semantics;
    let all: Vec<&Unit> = units.iter().collect();
    let mut metrics = calc.metrics(&all, semantics);
    let design = p.positive.and(reference::design(
        units
            .iter()
            .map(|u| u.votes.keys().map(String::as_str).collect()),
        s.rater_design,
    ));
    let by_rater = match &design {
        Some(d) if d.rater_design == reference::Design::Crossed && d.voters > 1 => {
            calc.by_rater(units, semantics)
        }
        _ => BTreeMap::new(),
    };
    for m in &mut metrics {
        let values = by_rater.values().filter_map(|v| v.get(m.metric).copied());
        m.rater_spread = values.fold(None, |acc, v| match acc {
            None => Some((v, v)),
            Some((lo, hi)) => Some((f64::min(lo, v), f64::max(hi, v))),
        });
    }
    let mut by = BTreeMap::new();
    if s.sections.by.is_some() {
        let strata: BTreeSet<&str> = units.iter().filter_map(|u| u.stratum.as_deref()).collect();
        for st in strata {
            let sub: Vec<&Unit> = units
                .iter()
                .filter(|u| u.stratum.as_deref() == Some(st))
                .collect();
            by.insert(st.to_owned(), calc.metrics(&sub, semantics));
        }
    }
    let mut reproducibility = BTreeMap::new();
    let n_clusters = units
        .iter()
        .map(|u| u.cluster.as_str())
        .collect::<BTreeSet<_>>()
        .len() as f64;
    let gen_values: Vec<Vec<u64>> = units
        .iter()
        .map(|u| {
            u.replicates
                .iter()
                .filter_map(|r| r.as_ref().ok().map(|v| v.to_bits()))
                .collect()
        })
        .collect();
    if let Some(d) = Disagreement::of(&gen_values) {
        reproducibility.insert("generation", repro_row(d, n_clusters, s.level));
    }
    let grade_values: Vec<Vec<u64>> = units
        .iter()
        .flat_map(|u| {
            u.grades
                .iter()
                .map(|g| g.iter().map(|v| v.to_bits()).collect())
        })
        .collect();
    if let Some(d) = Disagreement::of(&grade_values) {
        reproducibility.insert("grading", repro_row(d, n_clusters, s.level));
    }
    let n_insufficient_replicates = match s.estimand {
        Estimand::SingleShot => 0,
        Estimand::Majority(k) | Estimand::Any(k) | Estimand::All(k) => units
            .iter()
            .filter(|u| {
                let usable = u
                    .replicates
                    .iter()
                    .filter(|r| r.err().is_none_or(|m| s.missing.policy(m) != Policy::Drop))
                    .count();
                usable < k as usize
            })
            .count(),
    };
    let reference = p.positive.map(|_| ReferenceInfo {
        mode: s.reference,
        raters: p.raters,
        items_with_reference: units.iter().filter(|u| u.reference.is_some()).count(),
        model_derived_items: units.iter().filter(|u| u.model_reference).count(),
        inclusion_weighted_items: units.iter().filter(|u| u.inclusion_weighted).count(),
        design,
        by_rater,
    });
    let item_rows = if s.sections.items {
        units
            .iter()
            .map(|u| ItemRow {
                item_id: u.item_id.clone(),
                reference: u.reference,
                judged: calc.item_value(u, |_| None),
                replicates: u.replicates.len(),
                missing: u.replicates.iter().filter_map(|r| r.err()).collect(),
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Report {
        run: p.run_id.clone(),
        task: s.task.clone(),
        value_semantics: semantics,
        output_type: p.output_type,
        positive: p.positive,
        decision: p.rule,
        estimand: s.estimand.to_string(),
        reference,
        n_items: units.len(),
        n_judgments: p.n_judgments,
        n_missing: p.missing_by_reason.values().sum(),
        missing_by_reason: p.missing_by_reason.clone(),
        missing_policy: s.missing.table(),
        n_insufficient_replicates,
        level: s.level,
        seed: s.seed,
        metrics,
        reproducibility,
        by,
        items: item_rows,
        roc: s.sections.detail.then(|| p.roc()).flatten(),
        sdt: s.sections.detail.then(|| p.sdt()).flatten(),
        calibration: s.sections.detail.then(|| p.calibration()).flatten(),
        features: s
            .sections
            .features
            .then(|| p.features(crate::features::Thresholds::default(), s.prevalence)),
        proxy: s
            .proxy
            .as_deref()
            .map(|r| p.proxy_estimate(items, labels, r))
            .transpose()?,
        // A full cluster bootstrap: only the detail tier asks for it.
        expected_cost: s
            .cost
            .as_ref()
            .filter(|_| s.sections.detail)
            .zip(p.cost_parts())
            .map(|(c, parts)| crate::cost::summary(&parts, c, s.bootstrap, s.seed, s.level)),
    })
}

impl Prepared<'_> {
    fn calc(&self) -> Calc<'_> {
        Calc {
            settings: self.settings,
            binary: self.rule.is_some(),
            at_least: self.at_least,
            score_range: self.score_range,
        }
    }
}

fn repro_row(d: Disagreement, n_eff: f64, level: f64) -> ReproducibilityRow {
    ReproducibilityRow {
        pairwise_disagreement: d.pairwise,
        items_with_any_disagreement: d.any_item,
        items: d.items,
        upper_bound: d.upper_bound(n_eff.min(d.items as f64), level),
    }
}

struct Calc<'a> {
    settings: &'a Settings,
    binary: bool,
    at_least: Option<f64>,
    score_range: (f64, f64),
}

/// A metric of the default tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetricId {
    /// Share of items on which the judgment equals the reference.
    Accuracy,
    /// Share of positives judged positive.
    HitRate,
    /// Share of negatives judged positive.
    FalseAlarmRate,
    /// Share of positive judgments that are positive.
    Precision,
    /// Share of binary judgments that pass.
    PassRate,
    /// Mean judgment value of a score run.
    MeanScore,
    /// Share of score judgments at or above the threshold.
    RateAtLeast,
    /// Expected cost under the `[cost]` settings (D-041). Not a ratio of item parts:
    /// computed from [`Prepared::cost_parts`], never by `item_values`.
    ExpectedCost,
}

/// Serialized as [`MetricId::name`], the one place its identifier is written.
impl Serialize for MetricId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl MetricId {
    /// The metric's identifier in CLI output and JSON.
    pub fn name(self) -> &'static str {
        match self {
            MetricId::Accuracy => "accuracy",
            MetricId::HitRate => "hit_rate",
            MetricId::FalseAlarmRate => "false_alarm_rate",
            MetricId::Precision => "precision",
            MetricId::PassRate => "pass_rate",
            MetricId::MeanScore => "mean_score",
            MetricId::RateAtLeast => "rate_at_least",
            MetricId::ExpectedCost => "expected_cost",
        }
    }

    /// Whether larger values are better (false alarms are better when fewer).
    pub fn higher_is_better(self) -> bool {
        !matches!(self, MetricId::FalseAlarmRate | MetricId::ExpectedCost)
    }

    /// `higher_is_better` or `lower_is_better`, as comparisons report it.
    pub fn direction(self) -> &'static str {
        if self.higher_is_better() {
            "higher_is_better"
        } else {
            "lower_is_better"
        }
    }

    /// Whether filling a missing judgment with the best or worst value bounds the metric.
    /// Precision's denominator is the items judged positive, so a filled judgment moves
    /// the denominator and gives no bounds.
    pub fn has_missing_bounds(self) -> bool {
        !matches!(self, MetricId::Precision | MetricId::ExpectedCost)
    }

    /// The factor that orients a difference so that positive is better.
    pub fn sign(self) -> f64 {
        if self.higher_is_better() { 1.0 } else { -1.0 }
    }

    fn needs_reference(self) -> bool {
        matches!(
            self,
            MetricId::Accuracy
                | MetricId::HitRate
                | MetricId::FalseAlarmRate
                | MetricId::Precision
                | MetricId::ExpectedCost
        )
    }

    /// A rate whose two arms share one denominator, so paired-proportion methods apply.
    pub fn is_paired_proportion(self) -> bool {
        matches!(
            self,
            MetricId::Accuracy
                | MetricId::HitRate
                | MetricId::FalseAlarmRate
                | MetricId::PassRate
                | MetricId::RateAtLeast
        )
    }
}

/// The worst replicate value of a unit for a metric (`None`: no missing bounds).
type WorstFn<'a> = Box<dyn Fn(&Unit) -> Option<f64> + 'a>;
/// A unit's (numerator, denominator) for its item value.
type PartFn<'a> = Box<dyn Fn(&Unit, f64) -> Part + 'a>;

/// How a metric fills missing replicates and what each item contributes.
struct Definition<'a> {
    worst: WorstFn<'a>,
    part: PartFn<'a>,
    values: Values,
}

/// Which replicate values a metric reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Values {
    /// Judged values (0/1 shares) or raw scores.
    Judged,
    /// Shares at or above the score threshold.
    AtLeast,
}

/// One item's contribution to a ratio metric: numerator and denominator weight.
type Part = (f64, f64);

impl Calc<'_> {
    /// The item's value under the estimand, with missing replicates filled for a metric
    /// whose worst replicate value is `worst(unit)`.
    fn item_value(&self, u: &Unit, worst: impl Fn(&Unit) -> Option<f64>) -> Option<f64> {
        self.item_value_bound(u, &worst, Fill::Policy, Values::Judged)
    }

    fn item_value_bound(
        &self,
        u: &Unit,
        worst: &dyn Fn(&Unit) -> Option<f64>,
        bound: Fill,
        which: Values,
    ) -> Option<f64> {
        let binary = self.binary || which == Values::AtLeast;
        let (lo, hi) = if binary { (0.0, 1.0) } else { self.score_range };
        let reps = match which {
            Values::Judged => &u.replicates,
            Values::AtLeast => &u.at_least,
        };
        let w = worst(u).unwrap_or(lo);
        let b = if w == lo { hi } else { lo };
        let mut values = Vec::new();
        for r in reps {
            match r {
                Ok(v) => values.push(*v),
                Err(reason) => {
                    let system = reason.side() == Side::System;
                    let policy = match (bound, system) {
                        (Fill::Worst, true) => Policy::Worst,
                        (Fill::Best, true) => Policy::Best,
                        _ => self.settings.missing.policy(*reason),
                    };
                    values.extend(policy.fill(w, b));
                }
            }
        }
        if values.is_empty() {
            return None;
        }
        let v = if binary && self.settings.estimand != Estimand::SingleShot {
            // Replicates are 0/1 judgments here, grading replicates averaged.
            let c = values.iter().filter(|v| decided_positive(**v)).count() as u64;
            item_value(c, values.len() as u64, self.settings.estimand)?
        } else {
            mean(values.iter().copied())?
        };
        Some(v)
    }

    /// How a metric fills and counts items.
    fn definition(&self, id: MetricId) -> Definition<'_> {
        let s = |u: &Unit| u.reference.unwrap_or(0.0);
        let lo = self.score_range.0;
        match id {
            // Accuracy: the worst judgment is the one opposite to the reference majority.
            MetricId::Accuracy => Definition {
                worst: Box::new(move |u| Some(if s(u) >= 0.5 { 0.0 } else { 1.0 })),
                part: Box::new(move |u, v| {
                    let r = s(u);
                    (u.weight * (v * r + (1.0 - v) * (1.0 - r)), u.weight)
                }),
                values: Values::Judged,
            },
            MetricId::HitRate => Definition {
                worst: Box::new(|_| Some(0.0)),
                part: Box::new(move |u, v| (u.weight * s(u) * v, u.weight * s(u))),
                values: Values::Judged,
            },
            MetricId::FalseAlarmRate => Definition {
                worst: Box::new(|_| Some(1.0)),
                part: Box::new(move |u, v| (u.weight * (1.0 - s(u)) * v, u.weight * (1.0 - s(u)))),
                values: Values::Judged,
            },
            MetricId::Precision => Definition {
                worst: Box::new(|_| None),
                part: Box::new(move |u, v| (u.weight * s(u) * v, u.weight * v)),
                values: Values::Judged,
            },
            MetricId::PassRate => Definition {
                worst: Box::new(|_| Some(0.0)),
                part: Box::new(|u, v| (u.weight * v, u.weight)),
                values: Values::Judged,
            },
            MetricId::MeanScore => Definition {
                worst: Box::new(move |_| Some(lo)),
                part: Box::new(|u, v| (u.weight * v, u.weight)),
                values: Values::Judged,
            },
            MetricId::RateAtLeast => Definition {
                worst: Box::new(|_| Some(0.0)),
                part: Box::new(|u, v| (u.weight * v, u.weight)),
                values: Values::AtLeast,
            },
            MetricId::ExpectedCost => unreachable!("expected cost has no item parts"),
        }
    }

    /// The metrics a run supports, in report order.
    fn ids(&self, semantics: ValueSemantics) -> Vec<MetricId> {
        match semantics {
            ValueSemantics::Decision => vec![
                MetricId::Accuracy,
                MetricId::HitRate,
                MetricId::FalseAlarmRate,
                MetricId::Precision,
            ],
            ValueSemantics::Score if self.binary => vec![MetricId::PassRate],
            ValueSemantics::Score if self.at_least.is_some() => {
                vec![MetricId::MeanScore, MetricId::RateAtLeast]
            }
            ValueSemantics::Score => vec![MetricId::MeanScore],
        }
    }

    /// The units a metric counts: items with a reference for decision metrics.
    fn eligible<'u>(&self, id: MetricId, units: &[&'u Unit]) -> Vec<&'u Unit> {
        units
            .iter()
            .copied()
            .filter(|u| !id.needs_reference() || u.reference.is_some())
            .collect()
    }

    fn metrics(&self, units: &[&Unit], semantics: ValueSemantics) -> Vec<Metric> {
        let mut out: Vec<Metric> = self
            .ids(semantics)
            .into_iter()
            .map(|id| {
                let d = self.definition(id);
                self.ratio_of(
                    id.name(),
                    &self.eligible(id, units),
                    &*d.worst,
                    &*d.part,
                    d.values,
                )
            })
            .collect();
        if semantics == ValueSemantics::Decision
            && let Some(pi) = self.settings.prevalence
        {
            let (h, f) = (out[1].value, out[2].value);
            let converted = match (h, f) {
                (Some(h), Some(f)) if pi * h + (1.0 - pi) * f > 0.0 => {
                    Some(pi * h / (pi * h + (1.0 - pi) * f))
                }
                _ => None,
            };
            let n = out[1].n + out[2].n;
            out.push(Metric {
                metric: "precision_at_prevalence",
                value: converted,
                ci: None,
                interval: "none",
                resamples: None,
                missing_bounds: None,
                rater_spread: None,
                n,
            });
        }
        out
    }

    /// Each unit's (numerator, denominator) for a metric, filling system-side missing
    /// replicates as `bound` says; `None` for units with no value.
    fn parts(&self, id: MetricId, units: &[&Unit], bound: Fill) -> Vec<Option<Part>> {
        let d = self.definition(id);
        units
            .iter()
            .map(|u| {
                self.item_value_bound(u, &*d.worst, bound, d.values)
                    .map(|v| (d.part)(u, v))
            })
            .collect()
    }

    /// The point value of each reference metric with each rater's votes alone as the
    /// reference, for the raters who voted on at least one item.
    fn by_rater(
        &self,
        units: &[Unit],
        semantics: ValueSemantics,
    ) -> BTreeMap<String, BTreeMap<&'static str, f64>> {
        let raters: BTreeSet<&str> = units
            .iter()
            .flat_map(|u| u.votes.keys().map(String::as_str))
            .collect();
        // One copy of the units, whose reference is swapped for each rater's votes.
        let mut alone: Vec<Unit> = units.to_vec();
        raters
            .into_iter()
            .map(|rater| {
                for (a, u) in alone.iter_mut().zip(units) {
                    a.reference = u.votes.get(rater).copied();
                }
                let refs: Vec<&Unit> = alone.iter().collect();
                let values = self
                    .ids(semantics)
                    .into_iter()
                    .filter(|id| id.needs_reference())
                    .filter_map(|id| {
                        let parts = self.parts(id, &self.eligible(id, &refs), Fill::Policy);
                        let num = sum(parts.iter().flatten().map(|x| x.0));
                        let den = sum(parts.iter().flatten().map(|x| x.1));
                        (den > 0.0).then(|| (id.name(), num / den))
                    })
                    .collect();
                (rater.to_owned(), values)
            })
            .collect()
    }

    /// A ratio metric Σ numerator / Σ denominator over units, with its interval and
    /// missing bounds. `part(u, v)` gives a unit's numerator and denominator for its
    /// item value v.
    fn ratio_of(
        &self,
        metric: &'static str,
        units: &[&Unit],
        worst: &dyn Fn(&Unit) -> Option<f64>,
        part: &dyn Fn(&Unit, f64) -> Part,
        which: Values,
    ) -> Metric {
        let parts = |bound: Fill| -> (Vec<Option<Part>>, bool) {
            let mut any_system = false;
            let p = units
                .iter()
                .map(|u| {
                    let reps = match which {
                        Values::Judged => &u.replicates,
                        Values::AtLeast => &u.at_least,
                    };
                    any_system |= reps
                        .iter()
                        .any(|r| r.is_err_and(|reason| reason.side() == Side::System));
                    self.item_value_bound(u, worst, bound, which)
                        .map(|v| part(u, v))
                })
                .collect();
            (p, any_system)
        };
        let total = |p: &[Option<Part>]| {
            let num = sum(p.iter().flatten().map(|x| x.0));
            let den = sum(p.iter().flatten().map(|x| x.1));
            (den > 0.0).then(|| num / den)
        };
        let (point_parts, system_missing) = parts(Fill::Policy);
        let value = total(&point_parts);
        let n = sum(point_parts.iter().flatten().map(|x| x.1));
        let missing_bounds = if system_missing && worst(units[0]).is_some() {
            match (total(&parts(Fill::Worst).0), total(&parts(Fill::Best).0)) {
                (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
                _ => None,
            }
        } else {
            None
        };
        // A rate: Wilson when every unit is one item with unit weight and an integral
        // contribution; otherwise the cluster bootstrap, except where it cannot give an
        // interval with meaning (a rate of 0 or 1, where every resample is the same, or
        // fewer than five units) and where pass or fail is decided: there the exact
        // interval on the effective number of items. Other metrics: the bootstrap.
        let integral = point_parts
            .iter()
            .flatten()
            .all(|(a, d)| a.fract() == 0.0 && d.fract() == 0.0);
        let independent = units
            .iter()
            .all(|u| u.weight == 1.0 && u.cluster == u.item_id && u.replicates.len() <= 1);
        let rate = self.binary || which == Values::AtLeast;
        // Wilson reads no clusters, so they are grouped only for the other intervals.
        let groups: Vec<Vec<usize>> =
            if rate && integral && independent && !self.settings.exact_rates {
                Vec::new()
            } else {
                let mut clusters: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
                for (i, u) in units.iter().enumerate() {
                    clusters.entry(u.cluster.as_str()).or_default().push(i);
                }
                clusters.into_values().collect()
            };
        // Only units that carry the metric's denominator can move the ratio.
        let carrying = groups
            .iter()
            .filter(|g| g.iter().any(|&i| point_parts[i].is_some_and(|p| p.1 > 0.0)))
            .count();
        let at_edge = value.is_some_and(|v| v == 0.0 || v == 1.0);
        let exact = rate
            && (self.settings.exact_rates
                || (!(integral && independent) && (at_edge || carrying < bootstrap::MIN_UNITS)));
        let (ci, interval, resamples) = if value.is_none() {
            (None, "none", None)
        } else if exact {
            // The effective number of items is Kish's over the clusters' denominators:
            // the item count when every item is its own cluster with unit weight, the
            // cluster count when clusters are equal.
            let totals: Vec<f64> = groups
                .iter()
                .map(|g| sum(g.iter().filter_map(|&i| point_parts[i].map(|p| p.1))))
                .collect();
            let (s1, s2) = (
                sum(totals.iter().copied()),
                sum(totals.iter().map(|d| d * d)),
            );
            let n_eff = s1 * s1 / s2;
            let k = value.unwrap_or_default() * n_eff;
            (
                Some(clopper_pearson(k, n_eff, self.settings.level)),
                "clopper_pearson",
                None,
            )
        } else if rate && integral && independent {
            let k = sum(point_parts.iter().flatten().map(|x| x.0));
            (
                Some(wilson_two_sided(k, n, self.settings.level)),
                "wilson",
                None,
            )
        } else {
            if carrying < bootstrap::MIN_UNITS {
                return Metric {
                    metric,
                    value,
                    ci: None,
                    interval: "too_few_units",
                    resamples: None,
                    missing_bounds,
                    rater_spread: None,
                    n,
                };
            }
            let mut rng = Rng::new(self.settings.seed);
            let (reps, _) = bootstrap::replicates(
                groups.len(),
                None::<&Strata>,
                self.settings.bootstrap,
                &mut rng,
                |drawn| {
                    let picked = drawn
                        .iter()
                        .flat_map(|&g| groups[g].iter().map(|&i| point_parts[i]));
                    let (mut num, mut den) = (Running::default(), Running::default());
                    for p in picked.flatten() {
                        num.add(p.0);
                        den.add(p.1);
                    }
                    let d = den.total();
                    (d > 0.0).then(|| num.total() / d)
                },
            );
            (
                bootstrap::percentile_interval(reps, self.settings.level),
                "bootstrap",
                Some(self.settings.bootstrap),
            )
        };
        Metric {
            metric,
            value,
            ci,
            interval,
            resamples,
            missing_bounds,
            rater_spread: None,
            n,
        }
    }
}

/// Which way system-side missing replicates are filled when reading item values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// The missing policy.
    Policy,
    /// The worst judgment for the metric.
    Worst,
    /// The best judgment for the metric.
    Best,
}

/// One item's contribution to a metric.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemValue {
    /// Cluster the item belongs to.
    pub cluster: String,
    /// The item's numerator of the metric.
    pub numerator: f64,
    /// The item's denominator of the metric.
    pub denominator: f64,
}

impl Prepared<'_> {
    /// Value semantics of the run.
    pub fn semantics(&self) -> ValueSemantics {
        self.semantics
    }

    /// Output type of the judgments.
    pub fn output_type(&self) -> OutputType {
        self.output_type
    }

    /// The settings the run was prepared with.
    pub fn settings(&self) -> &Settings {
        self.settings
    }

    /// The `[cost]` settings, if any.
    pub fn cost(&self) -> Option<&crate::config::Cost> {
        self.settings.cost.as_ref()
    }

    /// Decision rule in effect, if any.
    pub fn rule(&self) -> Option<Rule> {
        self.rule
    }

    /// Positive definition in effect, if any.
    pub fn positive(&self) -> Option<Rule> {
        self.positive
    }

    /// Number of raters across the run's labels.
    pub fn raters(&self) -> usize {
        self.raters
    }

    /// Missing judgments per reason.
    pub fn missing_by_reason(&self) -> &BTreeMap<MissingReason, u64> {
        &self.missing_by_reason
    }

    /// Number of missing judgments, every reason counted.
    pub fn n_missing(&self) -> u64 {
        self.missing_by_reason.values().sum()
    }

    /// System-side missing judgments and judged items, for the missing-rate check.
    pub fn system_missing(&self) -> (u64, u64) {
        let missing = self
            .missing_by_reason
            .iter()
            .filter(|(r, _)| r.side() == Side::System)
            .map(|(_, n)| n)
            .sum();
        (missing, self.n_judgments as u64)
    }

    /// The metrics this run supports, in report order.
    pub fn metric_ids(&self) -> Vec<MetricId> {
        self.calc().ids(self.semantics)
    }

    /// Whether every item is its own unit with weight 1, one replicate and a reference
    /// that is all or nothing, so counts are integral and independent.
    pub fn independent(&self) -> bool {
        self.units.iter().all(|u| {
            u.weight == 1.0
                && u.cluster == u.item_id
                && u.replicates.len() <= 1
                && u.reference.is_none_or(|r| r == 0.0 || r == 1.0)
        })
    }

    /// Each eligible item's contribution to a metric.
    pub fn item_values(&self, id: MetricId, fill: Fill) -> BTreeMap<String, ItemValue> {
        let calc = self.calc();
        let all: Vec<&Unit> = self.units.iter().collect();
        let units = calc.eligible(id, &all);
        let bound = fill;
        units
            .iter()
            .zip(calc.parts(id, &units, bound))
            .filter_map(|(u, p)| {
                p.map(|(numerator, denominator)| {
                    (
                        u.item_id.clone(),
                        ItemValue {
                            cluster: u.cluster.clone(),
                            numerator,
                            denominator,
                        },
                    )
                })
            })
            .collect()
    }

    /// Items with a missing judgment that the missing policy counts as the worst.
    pub fn missing_worst(&self) -> BTreeSet<String> {
        self.units
            .iter()
            .filter(|u| {
                u.replicates.iter().any(|r| {
                    r.is_err_and(|reason| {
                        self.settings.missing.policy(reason) == crate::missing::Policy::Worst
                    })
                })
            })
            .map(|u| u.item_id.clone())
            .collect()
    }

    /// Each item's judged value under the missing policy, with a system-side gap filled
    /// as accuracy fills it, and its reference share.
    pub fn judged(&self) -> BTreeMap<String, (f64, Option<f64>)> {
        let calc = self.calc();
        self.units
            .iter()
            .filter_map(|u| {
                let worst = |u: &Unit| u.reference.map(|s| if s >= 0.5 { 0.0 } else { 1.0 });
                calc.item_value(u, worst)
                    .map(|v| (u.item_id.clone(), (v, u.reference)))
            })
            .collect()
    }
}
