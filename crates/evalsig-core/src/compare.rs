//! Paired comparison of a baseline and candidate runs on their common items (metrics
//! spec §7). Differences are oriented so that positive is better.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bootstrap::{self, Strata};
use crate::interval::newcombe;
use crate::numeric::{Rng, mean, norm_ppf, sum, variance};
use crate::paired::{self, Discordant, Transition, Verdict};
use crate::report::{Fill, ItemValue, MetricId, Prepared};

/// The change of one metric from the baseline arm to the candidate arm.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricDelta {
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
    /// Method of the interval: `tango`, `newcombe`, `welch`, `bootstrap`, `too_few_units`
    /// or `none`.
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
    /// Test behind `p_value`: `mcnemar_exact`, `welch_t`, `bootstrap` or `none`.
    pub p_method: &'static str,
    /// Verdict on the difference given its interval and the minimum effect.
    pub verdict: Verdict,
}

/// A comparison of two arms over every metric both support.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    /// Number of paired items in the comparison.
    pub n_used: usize,
    /// Number of items present in only one arm.
    pub n_unpaired: u64,
    /// One entry per metric both arms support.
    pub metrics: Vec<MetricDelta>,
    /// Items moving between correct and wrong; present for a binary decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
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
}

fn values_of(p: &Prepared<'_>, id: MetricId) -> Filled {
    [Fill::Policy, Fill::Worst, Fill::Best].map(|f| p.item_values(id, f))
}

impl<'p, 'a> Baseline<'p, 'a> {
    /// Compute the per-metric item values of a prepared run once for reuse across
    /// comparisons.
    pub fn new(prepared: &'p Prepared<'a>) -> Self {
        let values = prepared
            .metric_ids()
            .into_iter()
            .map(|id| (id, values_of(prepared, id)))
            .collect();
        Baseline {
            prepared,
            values,
            judged: prepared.judged(),
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

/// A metric's difference with its §7.1 verdict.
fn finish(
    id: MetricId,
    (base, cand): (Option<f64>, Option<f64>),
    missing_bounds: Option<(f64, f64)>,
    o: &Options,
    (ci, interval, resamples, p_value, p_method): Inference,
) -> MetricDelta {
    let delta = base.zip(cand).map(|(x, y)| id.sign() * (y - x));
    let min_effect = o.min_effect.get(id.name()).copied().unwrap_or(0.0);
    MetricDelta {
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
    }
}

/// The lower and upper ends of two differences, when both exist.
fn bounds(x: Option<f64>, y: Option<f64>) -> Option<(f64, f64)> {
    x.zip(y).map(|(x, y)| (x.min(y), x.max(y)))
}

/// Compare the baseline with one candidate on the items both have.
pub fn compare(base: &Baseline<'_, '_>, b: &Prepared<'_>, o: &Options) -> Comparison {
    let a = base.prepared;
    let ids = shared_ids(a, b);
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
    let independent = a.independent() && b.independent() && a.rule().is_some();
    let system_missing = a.system_missing().0 + b.system_missing().0 > 0;
    let metrics = ids
        .iter()
        .map(|&id| {
            metric_delta(
                id,
                &base.values[&id],
                &values_of(b, id),
                &paired,
                independent,
                system_missing,
                o,
            )
        })
        .collect();
    let transition = a.positive().map(|_| {
        let mut t = Transition::default();
        for item in &paired {
            let ((va, s), (vb, _)) = (base.judged[*item], judged_b[*item]);
            if let Some(s) = s {
                t.add(s, va >= 0.5, vb >= 0.5);
            }
        }
        t
    });
    Comparison {
        n_used: paired.len(),
        n_unpaired,
        metrics,
        transition,
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
        let both: Vec<&str> = items
            .iter()
            .copied()
            .filter(|i| x.contains_key(*i) && y.contains_key(*i))
            .collect();
        ratio(x, &both)
            .zip(ratio(y, &both))
            .map(|(x, y)| sign * (y - x))
    };
    let missing_bounds = if system_missing && id != MetricId::Precision {
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
        let ci = paired::tango(d, norm_ppf(0.5 + o.level / 2.0))
            .map(|(lo, hi)| if sign > 0.0 { (lo, hi) } else { (-hi, -lo) });
        (
            ci,
            "tango",
            None,
            Some(paired::mcnemar(only_a, only_b).0),
            "mcnemar_exact",
        )
    } else if clusters.len() < bootstrap::MIN_UNITS {
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
        let p = delta.and_then(|d| bootstrap::p_value(&reps, d));
        (
            bootstrap::percentile_interval(reps, o.level),
            "bootstrap",
            Some(o.bootstrap),
            p,
            "bootstrap",
        )
    };
    finish(id, (base, cand), missing_bounds, o, inference)
}

/// Welch's t interval for the difference of two means, with its two-sided p-value.
fn welch(a: &[f64], b: &[f64], level: f64) -> Option<((f64, f64), f64)> {
    use statrs::distribution::{ContinuousCDF, StudentsT};
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
    let t = StudentsT::new(0.0, 1.0, df).ok()?;
    let q = t.inverse_cdf(0.5 + level / 2.0);
    let d = mb - ma;
    let p = 2.0 * t.sf((d / se).abs());
    Some(((d - q * se, d + q * se), p))
}

/// Each cluster's (numerator, denominator) totals.
fn cluster_totals(v: &BTreeMap<String, ItemValue>) -> Vec<[f64; 2]> {
    let mut c: BTreeMap<&str, [f64; 2]> = BTreeMap::new();
    for x in v.values() {
        let e = c.entry(x.cluster.as_str()).or_default();
        e[0] += x.numerator;
        e[1] += x.denominator;
    }
    c.into_values().collect()
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
    let missing_bounds = (has_system_missing && id != MetricId::Precision)
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
        let ci = newcombe(
            k(&pb),
            n(&pb),
            k(&pa),
            n(&pa),
            norm_ppf(0.5 + o.level / 2.0),
        );
        (Some(orient(ci)), "newcombe", None, None, "none")
    } else if id == MetricId::MeanScore && unit(&pa) && unit(&pb) {
        let points = |v: &BTreeMap<String, ItemValue>| -> Vec<f64> {
            v.values().map(|x| x.numerator).collect()
        };
        match welch(&points(&pa), &points(&pb), o.level) {
            Some((ci, p)) => (Some(orient(ci)), "welch", None, Some(p), "welch_t"),
            None => TOO_FEW_UNITS,
        }
    } else {
        let (ca, cb) = (cluster_totals(&pa), cluster_totals(&pb));
        if ca.len() < bootstrap::MIN_UNITS || cb.len() < bootstrap::MIN_UNITS {
            TOO_FEW_UNITS
        } else {
            // The arms are independent, so each draws its own clusters.
            let (mut ra, mut rb) = (Rng::new(o.seed), Rng::new(o.seed.wrapping_add(1)));
            let ratio_of = |c: &[[f64; 2]], drawn: &[usize]| {
                let (k, n) = drawn
                    .iter()
                    .fold((0.0, 0.0), |(k, n), &g| (k + c[g][0], n + c[g][1]));
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
    finish(id, (base, cand), missing_bounds, o, inference)
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
            let (pa, pb) = (&base.values[&id][0], &b.item_values(id, Fill::Policy));
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
            let mut rng = Rng::new(o.seed);
            for i in (1..ds.len()).rev() {
                ds.swap(i, rng.below(i as u64 + 1) as usize);
            }
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
