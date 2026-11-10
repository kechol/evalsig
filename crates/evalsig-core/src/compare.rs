//! Paired comparison of a baseline and candidate runs on their common items (metrics
//! spec §7). Differences are oriented so that positive is better.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bootstrap::{self, Strata};
use crate::config::Cost;
use crate::cost::{self, Clustered, CostPart, Mix, Sums};
use crate::interval::newcombe;
use crate::numeric::{Rng, Running, mean, student_t, sum, two_sided_z, variance};
use crate::paired::{self, Discordant, Transition, Verdict};
use crate::replicates::decided_positive;
use crate::report::{Fill, ItemValue, MetricId, Prepared};
use statrs::distribution::ContinuousCDF;

/// The change of one metric from the baseline arm to the candidate arm.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricDelta {
    /// The metric, for code that needs its properties.
    #[serde(skip)]
    pub id: MetricId,
    /// Name of the metric.
    pub metric: &'static str,
    /// `higher_is_better` or `lower_is_better`.
    pub direction: &'static str,
    /// The metric's value in the baseline arm; `None` when it cannot be computed.
    pub baseline: Option<f64>,
    /// The metric's value in the candidate arm; `None` when it cannot be computed.
    pub candidate: Option<f64>,
    /// Candidate minus baseline, oriented by the metric as measured; `None` when either
    /// value is missing.
    pub delta: Option<f64>,
    /// Confidence interval of the difference; `None` when no interval could be formed.
    pub ci: Option<(f64, f64)>,
    /// Method of the interval: `tango`, `newcombe`, `welch`, `bootstrap`, `too_few_units`,
    /// `zero_variance` or `none`.
    pub interval: &'static str,
    /// Number of bootstrap resamples, present for a bootstrap interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resamples: Option<u32>,
    /// Minimum effect: differences smaller than this are not called improvements.
    pub min_effect: f64,
    /// The difference when system-side missing values are filled against the
    /// candidate (candidate worst, baseline best) and in its favour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_bounds: Option<(f64, f64)>,
    /// Two-sided p-value of the difference; `None` when the test could not be run.
    pub p_value: Option<f64>,
    /// Test behind `p_value`: `tango_score`, `welch_t`, `bootstrap` or `none`.
    pub p_method: &'static str,
    /// Verdict on the difference given its interval and the minimum effect.
    pub verdict: Verdict,
    /// For expected cost with a `[cost] by` stratum: how strata were combined and one
    /// row per stratum.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub by_stratum: Option<ByStratum>,
}

/// Expected cost per stratum (D-041).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ByStratum {
    /// How strata were combined into the overall value.
    pub mix: Mix,
    /// One row per stratum.
    pub strata: Vec<StratumDelta>,
}

impl ByStratum {
    /// The strata present.
    pub fn present(&self) -> Vec<Option<String>> {
        self.strata.iter().map(|s| s.stratum.clone()).collect()
    }
}

/// Expected cost in one stratum of both arms (D-041).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StratumDelta {
    /// The stratum's value; `None` for items without one.
    pub stratum: Option<String>,
    /// The stratum's production prevalence.
    pub prevalence: f64,
    /// Expected cost in the baseline arm.
    pub baseline: Option<f64>,
    /// Expected cost in the candidate arm.
    pub candidate: Option<f64>,
    /// Baseline minus candidate (positive is better).
    pub delta: Option<f64>,
    /// Paired cluster-bootstrap interval of the difference.
    pub ci: Option<(f64, f64)>,
}

/// A comparison of two arms over every metric both support.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    /// Number of paired items in the comparison.
    pub n_used: usize,
    /// Number of items present in only one arm.
    pub n_unpaired: u64,
    /// One entry per metric both arms support and the baseline holds values for.
    pub metrics: Vec<MetricDelta>,
    /// Items moving between correct and wrong; present for a binary decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Signal-detection deltas, when asked for (`compare --sdt`, D-042).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdt: Option<SdtChange>,
}

/// Settings of a comparison.
#[derive(Debug, Clone)]
pub struct Options {
    /// Confidence level of the intervals, in (0, 1).
    pub level: f64,
    /// Number of bootstrap resamples.
    pub bootstrap: u32,
    /// Seed of the random number generator for resampling.
    pub seed: u64,
    /// Minimum effect per metric name; metrics not listed use 0.
    pub min_effect: BTreeMap<String, f64>,
    /// Items left out of the comparison (inputs that changed between arms).
    pub excluded: BTreeSet<String>,
    /// When set, only these items are compared (a part of a split).
    pub scope: Option<BTreeSet<String>>,
    /// Add signal-detection deltas, with this fewest weighted items per cell for a
    /// classification (D-042).
    pub sdt_min_events: Option<f64>,
}

impl Options {
    /// Whether an item takes part: in the scope, if any, and not excluded.
    pub fn counts(&self, item: &str) -> bool {
        self.scope.as_ref().is_none_or(|s| s.contains(item)) && !self.excluded.contains(item)
    }
}

/// One arm's per-item values for a metric under the policy, worst and best fills.
type Filled = [BTreeMap<String, ItemValue>; 3];

/// A baseline's per-item values, computed once and shared by every candidate.
pub struct Baseline<'p, 'a> {
    prepared: &'p Prepared<'a>,
    values: BTreeMap<MetricId, Filled>,
    judged: BTreeMap<String, (f64, Option<f64>)>,
    cost: Option<BTreeMap<String, CostPart>>,
}

/// With `bounds` false the worst and best fills are left empty, for a comparison that
/// will not read them.
fn values_of(p: &Prepared<'_>, id: MetricId, bounds: bool) -> Filled {
    let v = |f| p.item_values(id, f);
    if bounds {
        [v(Fill::Policy), v(Fill::Worst), v(Fill::Best)]
    } else {
        [v(Fill::Policy), BTreeMap::new(), BTreeMap::new()]
    }
}

impl<'p, 'a> Baseline<'p, 'a> {
    /// Compute the per-metric item values of a prepared run once for reuse across
    /// comparisons.
    pub fn new(prepared: &'p Prepared<'a>) -> Self {
        Self::of(prepared, |_| true)
    }

    /// The same for the metric named `metric` alone, so a comparison computes only it.
    pub fn only(prepared: &'p Prepared<'a>, metric: &str) -> Self {
        Self::of(prepared, |id| id.name() == metric)
    }

    fn of(prepared: &'p Prepared<'a>, keep: impl Fn(MetricId) -> bool) -> Self {
        let values = prepared
            .metric_ids()
            .into_iter()
            .filter(|id| keep(*id))
            .map(|id| (id, values_of(prepared, id, id.has_missing_bounds())))
            .collect();
        Baseline {
            prepared,
            values,
            judged: prepared.judged(),
            cost: keep(MetricId::ExpectedCost)
                .then(|| prepared.cost_parts())
                .flatten(),
        }
    }
}

fn ratio(values: &BTreeMap<String, ItemValue>, items: &[&str]) -> Option<f64> {
    let den = sum(items.iter().map(|i| values[*i].denominator));
    (den > 0.0).then(|| sum(items.iter().map(|i| values[*i].numerator)) / den)
}

/// The metrics both runs support, in the baseline's order.
fn shared_ids(a: &Prepared<'_>, b: &Prepared<'_>) -> Vec<MetricId> {
    let b_ids = b.metric_ids();
    a.metric_ids()
        .into_iter()
        .filter(|m| b_ids.contains(m))
        .collect()
}

/// An interval and test: (interval, method, resamples, p-value, p-value method).
type Inference = (
    Option<(f64, f64)>,
    &'static str,
    Option<u32>,
    Option<f64>,
    &'static str,
);

const NO_INFERENCE: Inference = (None, "none", None, None, "none");
const TOO_FEW_UNITS: Inference = (None, "too_few_units", None, None, "none");
const ZERO_VARIANCE: Inference = (None, "zero_variance", None, None, "none");

/// A metric's difference, oriented so that positive is better, with its §7.1 verdict.
fn finish(
    id: MetricId,
    (base, cand, delta): (Option<f64>, Option<f64>, Option<f64>),
    missing_bounds: Option<(f64, f64)>,
    o: &Options,
    (ci, interval, resamples, p_value, p_method): Inference,
) -> MetricDelta {
    let min_effect = o.min_effect.get(id.name()).copied().unwrap_or(0.0);
    MetricDelta {
        id,
        metric: id.name(),
        direction: id.direction(),
        baseline: base,
        candidate: cand,
        delta,
        ci,
        interval,
        resamples,
        min_effect,
        missing_bounds,
        p_value,
        p_method,
        verdict: paired::verdict(delta.unwrap_or(0.0), delta.and(ci), min_effect),
        by_stratum: None,
    }
}

/// The lower and upper ends of two differences, when both exist.
fn bounds(x: Option<f64>, y: Option<f64>) -> Option<(f64, f64)> {
    x.zip(y).map(|(x, y)| (x.min(y), x.max(y)))
}

/// Compare the baseline with one candidate on the items both have, on the metrics the
/// baseline holds values for.
pub fn compare(base: &Baseline<'_, '_>, b: &Prepared<'_>, o: &Options) -> Comparison {
    let a = base.prepared;
    let ids: Vec<MetricId> = shared_ids(a, b)
        .into_iter()
        .filter(|id| base.values.contains_key(id))
        .collect();
    let judged_b = b.judged();
    let in_scope = |k: &&String| o.scope.as_ref().is_none_or(|s| s.contains(*k));
    let in_both: BTreeSet<&str> = base
        .judged
        .keys()
        .filter(in_scope)
        .filter(|k| judged_b.contains_key(*k))
        .map(String::as_str)
        .collect();
    let paired: BTreeSet<&str> = in_both.iter().copied().filter(|k| o.counts(k)).collect();
    let union = base
        .judged
        .keys()
        .chain(judged_b.keys())
        .filter(in_scope)
        .collect::<BTreeSet<_>>()
        .len();
    let n_unpaired = (union - in_both.len()) as u64;
    let independent = a.independent() && b.independent();
    let system_missing = a.system_missing().0 + b.system_missing().0 > 0;
    let mut metrics: Vec<MetricDelta> = ids
        .iter()
        .map(|&id| {
            metric_delta(
                id,
                &base.values[&id],
                &values_of(b, id, system_missing && id.has_missing_bounds()),
                &paired,
                independent,
                system_missing,
                o,
            )
        })
        .collect();
    if let (Some(pa), Some(pb), Some(c)) = (&base.cost, b.cost_parts(), a.cost()) {
        metrics.push(cost_delta(pa, &pb, c, &paired, o));
    }
    let transition = a.positive().map(|_| {
        Transition::of(paired.iter().filter_map(|item| {
            let ((va, s), (vb, _)) = (base.judged[*item], judged_b[*item]);
            s.map(|s| (s, decided_positive(va), decided_positive(vb)))
        }))
    });
    let sdt = o
        .sdt_min_events
        .and_then(|k| sdt_change(a, b, &paired, o, k));
    Comparison {
        n_used: paired.len(),
        n_unpaired,
        metrics,
        transition,
        sdt,
    }
}

/// `system_missing`: either arm has missing system judgments, so the difference gets
/// bounds from the worst and best fillings.
fn metric_delta(
    id: MetricId,
    va: &Filled,
    vb: &Filled,
    paired: &BTreeSet<&str>,
    independent: bool,
    system_missing: bool,
    o: &Options,
) -> MetricDelta {
    let sign = id.sign();
    let [pa, wa, ba] = va;
    let [pb, wb, bb] = vb;
    let items: Vec<&str> = paired
        .iter()
        .copied()
        .filter(|i| pa.contains_key(*i) && pb.contains_key(*i))
        .collect();
    let (base, cand) = (ratio(pa, &items), ratio(pb, &items));
    let delta = base.zip(cand).map(|(x, y)| sign * (y - x));
    // Bounds pair a worst-filled arm with a best-filled one: against the candidate
    // and in its favour.
    let oriented = |x: &BTreeMap<String, ItemValue>, y: &BTreeMap<String, ItemValue>| {
        let both: Vec<&str> = x
            .keys()
            .map(String::as_str)
            .filter(|i| o.counts(i) && y.contains_key(*i))
            .collect();
        ratio(x, &both)
            .zip(ratio(y, &both))
            .map(|(x, y)| sign * (y - x))
    };
    let missing_bounds = if system_missing && id.has_missing_bounds() {
        bounds(oriented(ba, wb), oriented(wa, bb))
    } else {
        None
    };
    let integral = items.iter().all(|i| {
        let (x, y) = (&pa[*i], &pb[*i]);
        [x.numerator, x.denominator, y.numerator, y.denominator]
            .iter()
            .all(|v| v.fract() == 0.0)
    });
    let mut clusters: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for i in &items {
        clusters.entry(pa[*i].cluster.as_str()).or_default().push(i);
    }
    // Clusters that carry an arm's denominator: the others cannot move that arm's ratio.
    let carrying = |v: &BTreeMap<String, ItemValue>| {
        clusters
            .values()
            .filter(|is| is.iter().any(|i| v[*i].denominator > 0.0))
            .count()
    };
    let inference = if delta.is_none() {
        NO_INFERENCE
    } else if independent && integral && id.is_paired_proportion() {
        // Each item counts 0 or 1 in a shared denominator: discordant pairs decide.
        let counted: Vec<&str> = items
            .iter()
            .copied()
            .filter(|i| pa[*i].denominator > 0.0)
            .collect();
        let (mut only_a, mut only_b) = (0u64, 0u64);
        for i in &counted {
            match (pa[*i].numerator > 0.5, pb[*i].numerator > 0.5) {
                (true, false) => only_a += 1,
                (false, true) => only_b += 1,
                _ => {}
            }
        }
        let d = Discordant {
            only_a,
            only_b,
            n: counted.len() as u64,
        };
        let ci = paired::tango(d, two_sided_z(o.level))
            .map(|(lo, hi)| if sign > 0.0 { (lo, hi) } else { (-hi, -lo) });
        (ci, "tango", None, Some(paired::tango_p(d)), "tango_score")
    } else if carrying(pa).min(carrying(pb)) < bootstrap::MIN_UNITS {
        TOO_FEW_UNITS
    } else {
        // Paired cluster bootstrap: one draw of clusters serves both arms. Each
        // cluster's sums are taken once, so a resample adds cluster totals.
        let totals: Vec<[f64; 4]> = clusters
            .values()
            .map(|is| {
                let s = |f: &dyn Fn(&str) -> f64| sum(is.iter().map(|i| f(i)));
                [
                    s(&|i| pa[i].numerator),
                    s(&|i| pa[i].denominator),
                    s(&|i| pb[i].numerator),
                    s(&|i| pb[i].denominator),
                ]
            })
            .collect();
        let mut rng = Rng::new(o.seed);
        let (reps, _) = bootstrap::replicates(
            totals.len(),
            None::<&Strata>,
            o.bootstrap,
            &mut rng,
            |drawn| {
                let t = |k: usize| sum(drawn.iter().map(|&g| totals[g][k]));
                let (da, db) = (t(1), t(3));
                (da > 0.0 && db > 0.0).then(|| sign * (t(2) / db - t(0) / da))
            },
        );
        bootstrap_inference(reps, delta, o)
    };
    finish(id, (base, cand, delta), missing_bounds, o, inference)
}

/// The difference in expected cost on the paired items (D-041): one paired cluster
/// bootstrap gives the overall interval and each stratum's.
fn cost_delta(
    pa: &BTreeMap<String, CostPart>,
    pb: &BTreeMap<String, CostPart>,
    c: &Cost,
    paired: &BTreeSet<&str>,
    o: &Options,
) -> MetricDelta {
    let items: Vec<&str> = paired
        .iter()
        .copied()
        .filter(|i| pa.contains_key(*i) && pb.contains_key(*i))
        .collect();
    let groups = Clustered::new(&items, [pa, pb]);
    let strata = &groups.strata;
    let mix = cost::mix(strata, c);
    let overall = |s: &[Vec<Sums>; 2]| {
        let ec = |arm: &[Sums]| cost::combine(strata, arm, c, mix);
        ec(&s[0]).zip(ec(&s[1]))
    };
    let one = |s: &[Vec<Sums>; 2], g: usize| {
        let pi = c.prevalence_of(strata[g].as_deref());
        cost::stratum_value(&s[0][g], pi).zip(cost::stratum_value(&s[1][g], pi))
    };
    let diff = |p: Option<(f64, f64)>| p.map(|(x, y)| x - y);
    let all = groups.sums(0..groups.n_clusters());
    let point = overall(&all);
    let delta = diff(point);
    let reps = if delta.is_some() {
        groups.replicates(
            o.bootstrap,
            o.seed,
            c.by.is_some(),
            |s| diff(overall(s)),
            |s, g| diff(one(s, g)),
        )
    } else {
        vec![Vec::new(); strata.len() + 1]
    };
    let mut reps = reps.into_iter();
    let overall_reps = reps.next().expect("one list per statistic");
    let inference = if delta.is_none() {
        NO_INFERENCE
    } else if groups.n_clusters() < bootstrap::MIN_UNITS {
        TOO_FEW_UNITS
    } else {
        bootstrap_inference(overall_reps, delta, o)
    };
    let mut d = finish(
        MetricId::ExpectedCost,
        (point.map(|p| p.0), point.map(|p| p.1), delta),
        None,
        o,
        inference,
    );
    if c.by.is_some() {
        d.by_stratum = Some(ByStratum {
            mix,
            strata: strata
                .iter()
                .zip(reps)
                .enumerate()
                .map(|(g, (stratum, r))| {
                    let point = one(&all, g);
                    StratumDelta {
                        stratum: stratum.clone(),
                        prevalence: c.prevalence_of(stratum.as_deref()),
                        baseline: point.map(|p| p.0),
                        candidate: point.map(|p| p.1),
                        delta: diff(point),
                        ci: groups.stratum_ci(g, point.is_some(), r, o.level),
                    }
                })
                .collect(),
        });
    }
    d
}

/// The change of signal-detection quantities between two arms on their paired items
/// (D-042): candidate − baseline as measured, with paired cluster-bootstrap intervals.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SdtChange {
    /// The model behind d′ and c.
    pub assumption: &'static str,
    /// ΔH, ΔF, Δd′ and Δc.
    pub deltas: Vec<SdtDelta>,
    /// What the intervals say changed.
    pub change: SdtClass,
    /// The fewest weighted items each cell of each arm needs for a classification.
    pub min_events: f64,
    /// Number of bootstrap resamples, when intervals were formed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resamples: Option<u32>,
}

/// One signal-detection quantity in both arms.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SdtDelta {
    /// `hit_rate`, `false_alarm_rate`, `dprime` or `criterion`.
    pub metric: &'static str,
    /// Value in the baseline arm.
    pub baseline: f64,
    /// Value in the candidate arm.
    pub candidate: f64,
    /// Candidate minus baseline.
    pub delta: f64,
    /// Paired cluster-bootstrap interval of the difference.
    pub ci: Option<(f64, f64)>,
}

/// What changed between the arms, read from the intervals of Δd′ and Δc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SdtClass {
    /// Δd′'s interval lies above 0.
    DiscriminationImproved,
    /// Δd′'s interval lies below 0.
    DiscriminationWorsened,
    /// Δd′'s interval contains 0 and Δc's does not.
    ThresholdMoved,
    /// Neither interval excludes 0.
    Undetermined,
    /// A cell of either arm is below the minimum, or there are too few clusters.
    InsufficientData,
}

/// Per arm: hits, positives, false alarms, negatives (weighted).
type Counts = [f64; 4];

fn add(acc: &mut [Counts; 2], t: &[Counts; 2]) {
    for (x, y) in acc.iter_mut().flatten().zip(t.iter().flatten()) {
        *x += y;
    }
}

fn confusion(&[h, p, f, n]: &Counts, scales: (f64, f64)) -> crate::sdt::Confusion {
    crate::sdt::Confusion::scaled((h, p), (f, n), scales)
}

/// H, F, d′ and c of an arm's counts, with the arm's class scaling (as `report`'s d′).
fn sdt_of(t: &Counts, scales: (f64, f64)) -> Option<[f64; 4]> {
    let [h, p, f, n] = *t;
    if p <= 0.0 || n <= 0.0 {
        return None;
    }
    let (d, c) = confusion(t, scales).dprime_and_criterion();
    Some([h / p, f / n, d, c])
}

/// ΔH, ΔF, Δd′ and Δc on the paired items (D-042). `None` without decisions and a
/// reference.
fn sdt_change(
    a: &Prepared<'_>,
    b: &Prepared<'_>,
    paired: &BTreeSet<&str>,
    o: &Options,
    min_events: f64,
) -> Option<SdtChange> {
    if a.positive().is_none() || a.semantics() != crate::records::ValueSemantics::Decision {
        return None;
    }
    let values = [a, b].map(|p| {
        [MetricId::HitRate, MetricId::FalseAlarmRate].map(|id| p.item_values(id, Fill::Policy))
    });
    // An item counts when both arms give it the same classes: the missing policy can
    // drop an item from one arm only.
    let in_both = |i: &str| {
        (0..2).all(|c| values[0][c].contains_key(i) == values[1][c].contains_key(i))
            && (0..2).any(|c| values[0][c].contains_key(i))
    };
    let items: BTreeSet<&str> = paired.iter().copied().filter(|i| in_both(i)).collect();
    let scales = [a, b].map(|p| p.class_scales_of(|i| items.contains(i)));
    let counts = |arm: usize, i: &str| -> Counts {
        let part = |v: &BTreeMap<String, ItemValue>| {
            v.get(i)
                .map_or((0.0, 0.0), |x| (x.numerator, x.denominator))
        };
        let [(h, p), (f, n)] = values[arm].each_ref().map(part);
        [h, p, f, n]
    };
    let mut clusters: BTreeMap<&str, [Counts; 2]> = BTreeMap::new();
    for i in &items {
        let [hv, fv] = &values[0];
        let Some(first) = hv.get(*i).or_else(|| fv.get(*i)) else {
            continue;
        };
        add(
            clusters.entry(first.cluster.as_str()).or_default(),
            &[counts(0, i), counts(1, i)],
        );
    }
    let totals: Vec<[Counts; 2]> = clusters.into_values().collect();
    let sum_of = |drawn: &mut dyn Iterator<Item = usize>| {
        let mut out = [[0.0; 4]; 2];
        drawn.for_each(|g| add(&mut out, &totals[g]));
        out
    };
    let points = |t: &[Counts; 2]| sdt_of(&t[0], scales[0]).zip(sdt_of(&t[1], scales[1]));
    let all = sum_of(&mut (0..totals.len()));
    let (pa, pb) = points(&all)?;
    let enough = totals.len() >= bootstrap::MIN_UNITS;
    let reps = if enough {
        let mut rng = Rng::new(o.seed);
        bootstrap::replicates_many(totals.len(), None, o.bootstrap, &mut rng, 4, |drawn| {
            let d = points(&sum_of(&mut drawn.iter().copied()))
                .map(|(x, y)| std::array::from_fn::<f64, 4, _>(|k| y[k] - x[k]));
            std::array::from_fn::<Option<f64>, 4, _>(|k| d.map(|d| d[k]))
        })
    } else {
        vec![Vec::new(); 4]
    };
    let deltas: Vec<SdtDelta> = ["hit_rate", "false_alarm_rate", "dprime", "criterion"]
        .into_iter()
        .zip(reps)
        .enumerate()
        .map(|(k, (metric, r))| SdtDelta {
            metric,
            baseline: pa[k],
            candidate: pb[k],
            delta: pb[k] - pa[k],
            ci: bootstrap::percentile_interval(r, o.level),
        })
        .collect();
    // The weighted cells on the item scale, as the log-linear correction counts them.
    let fewest = (0..2)
        .map(|arm| confusion(&all[arm], scales[arm]).fewest())
        .fold(f64::INFINITY, f64::min);
    let change = if !enough || fewest < min_events {
        SdtClass::InsufficientData
    } else {
        match (deltas[2].ci, deltas[3].ci) {
            (Some((lo, _)), _) if lo > 0.0 => SdtClass::DiscriminationImproved,
            (Some((_, hi)), _) if hi < 0.0 => SdtClass::DiscriminationWorsened,
            (Some(_), Some((lo, hi))) if lo > 0.0 || hi < 0.0 => SdtClass::ThresholdMoved,
            _ => SdtClass::Undetermined,
        }
    };
    Some(SdtChange {
        assumption: crate::sdt::ASSUMPTION,
        deltas,
        change,
        min_events,
        resamples: enough.then_some(o.bootstrap),
    })
}

/// The percentile interval and p-value of bootstrap replicates of a difference.
fn bootstrap_inference(reps: Vec<f64>, delta: Option<f64>, o: &Options) -> Inference {
    let p = delta.and_then(|d| bootstrap::p_value(&reps, d));
    (
        bootstrap::percentile_interval(reps, o.level),
        "bootstrap",
        Some(o.bootstrap),
        p,
        "bootstrap",
    )
}

/// Welch's t interval for the difference of two means, with its two-sided p-value.
fn welch(a: &[f64], b: &[f64], level: f64) -> Option<((f64, f64), f64)> {
    let (na, nb) = (a.len() as f64, b.len() as f64);
    if na < 2.0 || nb < 2.0 {
        return None;
    }
    let (ma, mb) = (mean(a.iter().copied())?, mean(b.iter().copied())?);
    let (sa, sb) = (variance(a, 1) / na, variance(b, 1) / nb);
    let se = (sa + sb).sqrt();
    if se == 0.0 {
        return None;
    }
    let df = (sa + sb).powi(2) / (sa * sa / (na - 1.0) + sb * sb / (nb - 1.0));
    let t = student_t(df)?;
    let q = t.inverse_cdf(0.5 + level / 2.0);
    let d = mb - ma;
    let p = 2.0 * t.sf((d / se).abs());
    Some(((d - q * se, d + q * se), p))
}

/// Each cluster's (numerator, denominator) totals.
fn cluster_totals(v: &BTreeMap<String, ItemValue>) -> Vec<[f64; 2]> {
    let mut c: BTreeMap<&str, [Running; 2]> = BTreeMap::new();
    for x in v.values() {
        let e = c.entry(x.cluster.as_str()).or_default();
        e[0].add(x.numerator);
        e[1].add(x.denominator);
    }
    c.into_values()
        .map(|[k, n]| [k.total(), n.total()])
        .collect()
}

/// The pooled ratio Σ numerator / Σ denominator of item values; none when empty.
pub fn total_ratio(v: &BTreeMap<String, ItemValue>) -> Option<f64> {
    let den = sum(v.values().map(|x| x.denominator));
    (den > 0.0).then(|| sum(v.values().map(|x| x.numerator)) / den)
}

/// Compare two runs whose item sets need not overlap, as independent samples
/// (metrics spec §7.3): Newcombe's interval for rates of 0/1 items, Welch's t for
/// mean scores of unit-weight items, otherwise a bootstrap that resamples each arm's
/// clusters separately. Items in both arms count in both.
pub fn unpaired(a: &Prepared<'_>, b: &Prepared<'_>, o: &Options) -> Comparison {
    let independent = a.independent() && b.independent();
    let has_system_missing = a.system_missing().0 + b.system_missing().0 > 0;
    let metrics = shared_ids(a, b)
        .into_iter()
        .map(|id| unpaired_delta(id, a, b, o, independent, has_system_missing))
        .collect();
    Comparison {
        n_used: a
            .judged()
            .keys()
            .chain(b.judged().keys())
            .filter(|k| o.counts(k))
            .count(),
        n_unpaired: 0,
        metrics,
        transition: None,
        sdt: None,
    }
}

fn unpaired_delta(
    id: MetricId,
    a: &Prepared<'_>,
    b: &Prepared<'_>,
    o: &Options,
    independent: bool,
    has_system_missing: bool,
) -> MetricDelta {
    let sign = id.sign();
    let values = |p: &Prepared<'_>, fill| -> BTreeMap<String, ItemValue> {
        p.item_values(id, fill)
            .into_iter()
            .filter(|(k, _)| o.counts(k))
            .collect()
    };
    let (pa, pb) = (values(a, Fill::Policy), values(b, Fill::Policy));
    let (base, cand) = (total_ratio(&pa), total_ratio(&pb));
    let delta = base.zip(cand).map(|(x, y)| sign * (y - x));
    let missing_bounds = (has_system_missing && id.has_missing_bounds())
        .then(|| {
            let d = |x: Option<f64>, y: Option<f64>| x.zip(y).map(|(x, y)| sign * (y - x));
            let (wa, ba) = (
                total_ratio(&values(a, Fill::Worst)),
                total_ratio(&values(a, Fill::Best)),
            );
            let (wb, bb) = (
                total_ratio(&values(b, Fill::Worst)),
                total_ratio(&values(b, Fill::Best)),
            );
            bounds(d(ba, wb), d(wa, bb))
        })
        .flatten();
    let integral = pa
        .values()
        .chain(pb.values())
        .all(|v| v.numerator.fract() == 0.0 && v.denominator.fract() == 0.0);
    let unit = |v: &BTreeMap<String, ItemValue>| {
        v.iter()
            .all(|(k, x)| x.denominator == 1.0 && x.cluster == *k)
    };
    let orient = |(lo, hi): (f64, f64)| if sign > 0.0 { (lo, hi) } else { (-hi, -lo) };
    let inference = if delta.is_none() {
        NO_INFERENCE
    } else if independent && integral && id.is_paired_proportion() {
        let k = |v: &BTreeMap<String, ItemValue>| sum(v.values().map(|x| x.numerator));
        let n = |v: &BTreeMap<String, ItemValue>| sum(v.values().map(|x| x.denominator));
        // newcombe gives p_first − p_second; candidate minus baseline here.
        let ci = newcombe(k(&pb), n(&pb), k(&pa), n(&pa), two_sided_z(o.level));
        (Some(orient(ci)), "newcombe", None, None, "none")
    } else if id == MetricId::MeanScore && unit(&pa) && unit(&pb) {
        let points = |v: &BTreeMap<String, ItemValue>| -> Vec<f64> {
            v.values().map(|x| x.numerator).collect()
        };
        let (a, b) = (points(&pa), points(&pb));
        match welch(&a, &b, o.level) {
            Some((ci, p)) => (Some(orient(ci)), "welch", None, Some(p), "welch_t"),
            None if a.len() < 2 || b.len() < 2 => TOO_FEW_UNITS,
            None => ZERO_VARIANCE,
        }
    } else {
        let (ca, cb) = (cluster_totals(&pa), cluster_totals(&pb));
        let carrying = |c: &[[f64; 2]]| c.iter().filter(|t| t[1] > 0.0).count();
        if carrying(&ca) < bootstrap::MIN_UNITS || carrying(&cb) < bootstrap::MIN_UNITS {
            TOO_FEW_UNITS
        } else {
            // The arms are independent, so each draws its own clusters.
            let (mut ra, mut rb) = (Rng::new(o.seed), Rng::new(o.seed.wrapping_add(1)));
            let ratio_of = |c: &[[f64; 2]], drawn: &[usize]| {
                let k = sum(drawn.iter().map(|&g| c[g][0]));
                let n = sum(drawn.iter().map(|&g| c[g][1]));
                (n > 0.0).then(|| k / n)
            };
            let (reps, _) = bootstrap::replicates(ca.len(), None, o.bootstrap, &mut ra, |da| {
                let db: Vec<usize> = (0..cb.len())
                    .map(|_| rb.below(cb.len() as u64) as usize)
                    .collect();
                ratio_of(&ca, da)
                    .zip(ratio_of(&cb, &db))
                    .map(|(x, y)| sign * (y - x))
            });
            let p = delta.and_then(|d| bootstrap::p_value(&reps, d));
            (
                bootstrap::percentile_interval(reps, o.level),
                "bootstrap",
                Some(o.bootstrap),
                p,
                "bootstrap",
            )
        }
    };
    finish(id, (base, cand, delta), missing_bounds, o, inference)
}

/// A paired difference monitored with a confidence sequence (metrics spec §7.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SequentialDelta {
    /// Name of the metric.
    pub metric: &'static str,
    /// `higher_is_better` or `lower_is_better`.
    pub direction: &'static str,
    /// Paired items in the order they were added.
    pub n: usize,
    /// Mean paired difference (candidate minus baseline), oriented so positive is better.
    pub delta: Option<f64>,
    /// The confidence sequence after the last item; valid at every earlier look too.
    pub confidence_sequence: Option<(f64, f64)>,
    /// Minimum effect used for the verdict.
    pub min_effect: f64,
    /// Verdict on the difference given the confidence sequence and the minimum effect.
    pub verdict: Verdict,
}

/// Paired differences of accuracy, H, F and the score metrics (metrics spec §7.2),
/// added in an order shuffled by the seed, with the confidence sequence of their
/// mean. A metric qualifies when every item it counts has denominator 1 and a value
/// in [0, 1], so the metric is the plain mean of the item values; soft or weighted
/// references, whose items carry fractional weights, have no sequence. An item counts
/// where both arms give it a positive denominator.
pub fn sequential(base: &Baseline<'_, '_>, b: &Prepared<'_>, o: &Options) -> Vec<SequentialDelta> {
    shared_ids(base.prepared, b)
        .into_iter()
        .filter_map(|id| {
            let sign = id.sign();
            let (pa, pb) = (&base.values.get(&id)?[0], &b.item_values(id, Fill::Policy));
            let value = |v: &ItemValue| v.numerator / v.denominator;
            let mut ds: Vec<f64> = pa
                .iter()
                .filter(|(k, x)| o.counts(k) && x.denominator > 0.0)
                .filter_map(|(k, x)| {
                    pb.get(k)
                        .filter(|y| y.denominator > 0.0)
                        .map(|y| (value(x), value(y)))
                })
                .map(|(x, y)| sign * (y - x))
                .collect();
            let plain_mean = pa
                .values()
                .chain(pb.values())
                .filter(|v| v.denominator > 0.0)
                .all(|v| v.denominator == 1.0 && (0.0..=1.0).contains(&v.numerator));
            if id == MetricId::Precision || !plain_mean {
                return None;
            }
            Rng::new(o.seed).shuffle(&mut ds);
            let delta = mean(ds.iter().copied());
            let cs = crate::sequential::difference(&ds, o.level).last().copied();
            let min_effect = o.min_effect.get(id.name()).copied().unwrap_or(0.0);
            Some(SequentialDelta {
                metric: id.name(),
                direction: id.direction(),
                n: ds.len(),
                delta,
                confidence_sequence: cs,
                min_effect,
                verdict: paired::verdict(delta.unwrap_or(0.0), delta.and(cs), min_effect),
            })
        })
        .collect()
}

/// A period's trend label against the baseline period: `drifted` when the interval of
/// the unpaired difference excludes 0, `stable` otherwise, `insufficient_data` without
/// an interval (metrics spec §7.3).
pub fn drift(d: &MetricDelta) -> Drift {
    match d.ci {
        None => Drift::InsufficientData,
        Some((lo, hi)) if lo > 0.0 || hi < 0.0 => Drift::Drifted,
        Some(_) => Drift::Stable,
    }
}

/// A period's trend label against the baseline period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Drift {
    /// The interval of the difference excludes 0.
    Drifted,
    /// The interval of the difference includes 0.
    Stable,
    /// No interval could be formed.
    InsufficientData,
}

/// The decision metric, Holm-adjusted p-values and best candidate of several
/// comparisons against one baseline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Multi {
    /// Name of the primary metric the summary is about.
    pub primary: &'static str,
    /// Holm-adjusted p-values of the primary metric, in candidate order; `None` when a
    /// candidate has no p-value.
    pub holm: Option<Vec<f64>>,
    /// Index of the candidate with the largest primary difference.
    pub best: Option<usize>,
}

/// Choose the primary metric (the requested one, else the first the runs share) and
/// summarize the comparisons. `Err` lists the available metrics.
pub fn summarize(
    comparisons: &[Comparison],
    requested: Option<&str>,
) -> Result<Multi, Vec<&'static str>> {
    let available: Vec<&'static str> = comparisons
        .first()
        .map(|c| c.metrics.iter().map(|m| m.metric).collect())
        .unwrap_or_default();
    let found = match requested {
        Some(r) => available.iter().find(|m| **m == r).copied(),
        None => available.first().copied(),
    };
    let primary = found.ok_or_else(|| available.clone())?;
    fn pick<'c>(c: &'c Comparison, primary: &str) -> Option<&'c MetricDelta> {
        c.metrics.iter().find(|m| m.metric == primary)
    }
    let ps: Option<Vec<f64>> = comparisons
        .iter()
        .map(|c| pick(c, primary).and_then(|m| m.p_value))
        .collect();
    let best = comparisons
        .iter()
        .enumerate()
        .filter_map(|(i, c)| pick(c, primary).and_then(|m| m.delta).map(|d| (i, d)))
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(i, _)| i);
    Ok(Multi {
        primary,
        holm: ps.and_then(|p| paired::holm(&p)),
        best,
    })
}
