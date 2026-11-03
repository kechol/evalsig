//! The default-tier report of one run (SPEC §6, metrics spec §0–§2, §6, §8, §9.2).
//!
//! Missing replicates are filled or dropped per metric under the missing policy,
//! replicates combine into one value per item under the estimand, and items (or their
//! clusters) are the resampling unit for intervals.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bootstrap::{self, Strata};
use crate::config::{Estimand, ReferenceMode};
use crate::interval::wilson_two_sided;
use crate::missing::{MissingPolicy, Policy, Side};
use crate::numeric::{Rng, sum};
use crate::records::{
    Item, Judgment, Label, LabelKind, MissingReason, OutputType, Run, Source, Value, ValueSemantics,
};
use crate::replicates::{Disagreement, item_value};
use crate::rules::Rule;

/// What a report needs besides the records.
#[derive(Debug, Clone)]
pub struct Settings {
    pub task: String,
    pub positive: Option<Rule>,
    pub decision: Option<Rule>,
    /// For score runs: report the share at or above this threshold.
    pub threshold: Option<f64>,
    pub estimand: Estimand,
    pub missing: MissingPolicy,
    pub reference: ReferenceMode,
    pub level: f64,
    pub bootstrap: u32,
    pub seed: u64,
    /// Production prevalence for the converted precision.
    pub prevalence: Option<f64>,
    pub by: Option<String>,
    pub items: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ReportError {
    #[error("run {0:?} has no judgments for task {1:?}")]
    NoJudgments(String, String),
    #[error("the {0} reference mode is not available in this version")]
    UnsupportedReference(&'static str),
    #[error("ordinal and score decisions need a decision rule (`decision` or --threshold)")]
    NoDecisionRule,
    #[error("{0} labels need a positive definition (`positive`)")]
    NoPositiveDefinition(&'static str),
    #[error("the {0} estimand needs pass or fail judgments; score runs need --threshold")]
    EstimandNeedsBinary(String),
}

/// One interval-bearing metric.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Metric {
    pub metric: &'static str,
    pub value: Option<f64>,
    pub ci: Option<(f64, f64)>,
    pub interval: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resamples: Option<u32>,
    /// System-side missing values filled with the worst and the best judgment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_bounds: Option<(f64, f64)>,
    pub n: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReproducibilityRow {
    pub pairwise_disagreement: f64,
    pub items_with_any_disagreement: f64,
    pub items: u64,
    pub upper_bound: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReferenceInfo {
    pub mode: ReferenceMode,
    pub raters: usize,
    pub items_with_reference: usize,
    pub model_derived_items: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemRow {
    pub item_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judged: Option<f64>,
    pub replicates: usize,
    pub missing: Vec<MissingReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub run: String,
    pub task: String,
    pub value_semantics: ValueSemantics,
    pub output_type: OutputType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positive: Option<Rule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<Rule>,
    pub estimand: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceInfo>,
    pub n_items: usize,
    pub n_judgments: usize,
    pub n_missing: u64,
    pub missing_by_reason: BTreeMap<MissingReason, u64>,
    pub missing_policy: BTreeMap<MissingReason, Policy>,
    pub n_insufficient_replicates: usize,
    pub level: f64,
    pub seed: u64,
    pub metrics: Vec<Metric>,
    pub reproducibility: BTreeMap<&'static str, ReproducibilityRow>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub by: BTreeMap<String, Vec<Metric>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemRow>,
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
}

/// The value a missing replicate takes under a policy; `None` drops it.
fn fill(policy: Policy, worst: f64, best: f64) -> Option<f64> {
    match policy {
        Policy::Worst => Some(worst),
        Policy::Best => Some(best),
        Policy::Drop => None,
    }
}

/// Positive share over the latest labels of one item under the reference mode.
fn reference_share(
    labels: &[&Label],
    positive: &Rule,
    mode: ReferenceMode,
) -> Option<(f64, usize, bool)> {
    let human: Vec<&Label> = labels
        .iter()
        .copied()
        .filter(|l| l.source == Source::Human)
        .collect();
    let pool = if human.is_empty() {
        labels.to_vec()
    } else {
        human
    };
    let model = pool.iter().all(|l| l.source == Source::Model);
    let value = |l: &Label| l.value.and_then(|v| positive.is_positive(v));
    if mode == ReferenceMode::Single
        && let Some(adj) = pool.iter().find(|l| l.kind() == LabelKind::Adjudication)
    {
        return value(adj).map(|p| (f64::from(u8::from(p)), 1, model));
    }
    let votes: Vec<bool> = pool
        .iter()
        .filter(|l| l.kind() != LabelKind::Adjudication && l.label_replicate() == 0)
        .filter_map(|l| value(l))
        .collect();
    if votes.is_empty() {
        return None;
    }
    let share = votes.iter().filter(|v| **v).count() as f64 / votes.len() as f64;
    let s = match mode {
        ReferenceMode::Soft => share,
        // Majority, and single without an adjudication: ties count one half.
        _ => match share.partial_cmp(&0.5) {
            Some(std::cmp::Ordering::Greater) => 1.0,
            Some(std::cmp::Ordering::Less) => 0.0,
            _ => 0.5,
        },
    };
    Some((s, votes.len(), model))
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
        let label_binary = labels
            .iter()
            .any(|l| matches!(l.value, Some(Value::Bool(_))));
        Some(match s.positive {
            Some(p) => p,
            None if label_binary => Rule::Equals(Value::Bool(true)),
            None => return Err(ReportError::NoPositiveDefinition("ordinal and score")),
        })
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
        for reps in gens.values() {
            let mut values = Vec::new();
            let mut reason = None;
            for j in reps {
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
                    replicates.push(Ok(sum(values.iter().copied()) / values.len() as f64));
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
            .and_then(|p| reference_share(&item_labels, p, s.reference));
        units.push(Unit {
            item_id: (*item_id).to_owned(),
            cluster: item.map_or((*item_id).to_owned(), |i| i.cluster_key().to_owned()),
            weight: item.map_or(1.0, |i| i.weight()),
            stratum: s.by.as_ref().and_then(|name| {
                item.and_then(|i| i.strata.as_ref())
                    .and_then(|m| m.get(name))
                    .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
            }),
            replicates,
            grades,
            at_least: at_least_reps,
            reference: reference.map(|r| r.0),
            model_reference: reference.is_some_and(|r| r.2),
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

/// Build the report of one run from its records.
pub fn build(
    run: &Run,
    judgments: &[&Judgment],
    items: &BTreeMap<&str, &Item>,
    labels: &[&Label],
    s: &Settings,
) -> Result<Report, ReportError> {
    let p = prepare(run, judgments, items, labels, s)?;
    let calc = p.calc();
    let units = &p.units;
    let semantics = p.semantics;
    let metrics = calc.metrics(units, semantics);
    let mut by = BTreeMap::new();
    if s.by.is_some() {
        let strata: BTreeSet<&str> = units.iter().filter_map(|u| u.stratum.as_deref()).collect();
        for st in strata {
            let sub: Vec<Unit> = units
                .iter()
                .filter(|u| u.stratum.as_deref() == Some(st))
                .cloned()
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
            .filter(|u| u.replicates.len() < k as usize)
            .count(),
    };
    let reference = p.positive.map(|_| ReferenceInfo {
        mode: s.reference,
        raters: p.raters,
        items_with_reference: units.iter().filter(|u| u.reference.is_some()).count(),
        model_derived_items: units.iter().filter(|u| u.model_reference).count(),
    });
    let item_rows = if s.items {
        units
            .iter()
            .map(|u| ItemRow {
                item_id: u.item_id.clone(),
                reference: u.reference,
                judged: calc.item_value(u, |_| None).map(|(v, _)| v),
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

/// Which way the system-side missing values are filled.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bound {
    Policy,
    Worst,
    Best,
}

struct Calc<'a> {
    settings: &'a Settings,
    binary: bool,
    at_least: Option<f64>,
    score_range: (f64, f64),
}

/// A metric of the default tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricId {
    Accuracy,
    HitRate,
    FalseAlarmRate,
    Precision,
    PassRate,
    MeanScore,
    RateAtLeast,
}

impl MetricId {
    pub fn name(self) -> &'static str {
        match self {
            MetricId::Accuracy => "accuracy",
            MetricId::HitRate => "hit_rate",
            MetricId::FalseAlarmRate => "false_alarm_rate",
            MetricId::Precision => "precision",
            MetricId::PassRate => "pass_rate",
            MetricId::MeanScore => "mean_score",
            MetricId::RateAtLeast => "rate_at_least",
        }
    }

    /// Whether larger values are better (false alarms are better when fewer).
    pub fn higher_is_better(self) -> bool {
        self != MetricId::FalseAlarmRate
    }

    fn needs_reference(self) -> bool {
        matches!(
            self,
            MetricId::Accuracy | MetricId::HitRate | MetricId::FalseAlarmRate | MetricId::Precision
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
    /// whose worst replicate value is `worst(unit)`; also whether a system-side
    /// replicate was missing.
    fn item_value(&self, u: &Unit, worst: impl Fn(&Unit) -> Option<f64>) -> Option<(f64, bool)> {
        self.item_value_bound(u, &worst, Bound::Policy, Values::Judged)
    }

    fn item_value_bound(
        &self,
        u: &Unit,
        worst: &dyn Fn(&Unit) -> Option<f64>,
        bound: Bound,
        which: Values,
    ) -> Option<(f64, bool)> {
        let binary = self.binary || which == Values::AtLeast;
        let (lo, hi) = if binary { (0.0, 1.0) } else { self.score_range };
        let reps = match which {
            Values::Judged => &u.replicates,
            Values::AtLeast => &u.at_least,
        };
        let w = worst(u).unwrap_or(lo);
        let b = if w == lo { hi } else { lo };
        let mut values = Vec::new();
        let mut system_missing = false;
        for r in reps {
            match r {
                Ok(v) => values.push(*v),
                Err(reason) => {
                    let system = reason.side() == Side::System;
                    system_missing |= system;
                    let policy = match (bound, system) {
                        (Bound::Worst, true) => Policy::Worst,
                        (Bound::Best, true) => Policy::Best,
                        _ => self.settings.missing.policy(*reason),
                    };
                    values.extend(fill(policy, w, b));
                }
            }
        }
        if values.is_empty() {
            return None;
        }
        let v = if binary && self.settings.estimand != Estimand::SingleShot {
            // Replicates are 0/1 judgments here (grading replicates averaged); round
            // a split grading to the nearer side for the @k rules.
            let c = values.iter().filter(|v| **v >= 0.5).count() as u64;
            item_value(c, values.len() as u64, self.settings.estimand)?
        } else {
            sum(values.iter().copied()) / values.len() as f64
        };
        Some((v, system_missing))
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
    fn eligible<'u>(&self, id: MetricId, units: &'u [Unit]) -> Vec<&'u Unit> {
        units
            .iter()
            .filter(|u| !id.needs_reference() || u.reference.is_some())
            .collect()
    }

    fn metrics(&self, units: &[Unit], semantics: ValueSemantics) -> Vec<Metric> {
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
                n,
            });
        }
        out
    }

    /// Each unit's (numerator, denominator) for a metric, filling system-side missing
    /// replicates as `bound` says; `None` for units with no value.
    fn parts(&self, id: MetricId, units: &[&Unit], bound: Bound) -> Vec<Option<Part>> {
        let d = self.definition(id);
        units
            .iter()
            .map(|u| {
                self.item_value_bound(u, &*d.worst, bound, d.values)
                    .map(|(v, _)| (d.part)(u, v))
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
        let parts = |bound: Bound| -> (Vec<Option<Part>>, bool) {
            let mut any_system = false;
            let p = units
                .iter()
                .map(|u| {
                    self.item_value_bound(u, worst, bound, which)
                        .map(|(v, sys)| {
                            any_system |= sys;
                            part(u, v)
                        })
                })
                .collect();
            (p, any_system)
        };
        let total = |p: &[Option<Part>]| {
            let num = sum(p.iter().flatten().map(|x| x.0));
            let den = sum(p.iter().flatten().map(|x| x.1));
            (den > 0.0).then(|| num / den)
        };
        let (point_parts, system_missing) = parts(Bound::Policy);
        let value = total(&point_parts);
        let n = sum(point_parts.iter().flatten().map(|x| x.1));
        let missing_bounds = if system_missing && worst(units[0]).is_some() {
            match (total(&parts(Bound::Worst).0), total(&parts(Bound::Best).0)) {
                (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
                _ => None,
            }
        } else {
            None
        };
        // Wilson when every unit is one item with unit weight and an integral
        // contribution; otherwise the cluster bootstrap.
        let integral = point_parts
            .iter()
            .flatten()
            .all(|(a, d)| a.fract() == 0.0 && d.fract() == 0.0);
        let independent = units
            .iter()
            .all(|u| u.weight == 1.0 && u.cluster == u.item_id && u.replicates.len() <= 1);
        let (ci, interval, resamples) = if value.is_none() {
            (None, "none", None)
        } else if (self.binary || which == Values::AtLeast) && integral && independent {
            let k = sum(point_parts.iter().flatten().map(|x| x.0));
            (
                Some(wilson_two_sided(k, n, self.settings.level)),
                "wilson",
                None,
            )
        } else {
            let mut clusters: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
            for (i, u) in units.iter().enumerate() {
                clusters.entry(u.cluster.as_str()).or_default().push(i);
            }
            let groups: Vec<Vec<usize>> = clusters.into_values().collect();
            if groups.len() < bootstrap::MIN_UNITS {
                return Metric {
                    metric,
                    value,
                    ci: None,
                    interval: "too_few_units",
                    resamples: None,
                    missing_bounds,
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
                    let (mut num, mut den) = (Vec::new(), Vec::new());
                    for p in picked.flatten() {
                        num.push(p.0);
                        den.push(p.1);
                    }
                    let d = sum(den);
                    (d > 0.0).then(|| sum(num) / d)
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
    pub cluster: String,
    pub numerator: f64,
    pub denominator: f64,
}

impl Prepared<'_> {
    pub fn semantics(&self) -> ValueSemantics {
        self.semantics
    }

    pub fn output_type(&self) -> OutputType {
        self.output_type
    }

    pub fn rule(&self) -> Option<Rule> {
        self.rule
    }

    pub fn positive(&self) -> Option<Rule> {
        self.positive
    }

    pub fn raters(&self) -> usize {
        self.raters
    }

    pub fn missing_by_reason(&self) -> &BTreeMap<MissingReason, u64> {
        &self.missing_by_reason
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
        let units = calc.eligible(id, &self.units);
        let bound = match fill {
            Fill::Policy => Bound::Policy,
            Fill::Worst => Bound::Worst,
            Fill::Best => Bound::Best,
        };
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

    /// Each item's judged value under the missing policy and its reference share.
    pub fn judged(&self) -> BTreeMap<String, (f64, Option<f64>)> {
        let calc = self.calc();
        self.units
            .iter()
            .filter_map(|u| {
                calc.item_value(u, |_| None)
                    .map(|(v, _)| (u.item_id.clone(), (v, u.reference)))
            })
            .collect()
    }
}
