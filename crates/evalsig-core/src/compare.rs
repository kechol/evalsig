//! Paired comparison of a baseline and candidate runs on their common items (metrics
//! spec §7). Differences are oriented so that positive is better.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bootstrap::{self, Strata};
use crate::interval::newcombe;
use crate::numeric::{Rng, mean, norm_ppf, sum};
use crate::paired::{self, Discordant, Transition, Verdict};
use crate::report::{Fill, ItemValue, MetricId, Prepared};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricDelta {
    pub metric: &'static str,
    pub direction: &'static str,
    pub baseline: Option<f64>,
    pub candidate: Option<f64>,
    pub delta: Option<f64>,
    pub ci: Option<(f64, f64)>,
    pub interval: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resamples: Option<u32>,
    pub min_effect: f64,
    /// The difference when system-side missing values are filled against the
    /// candidate (candidate worst, baseline best) and in its favour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_bounds: Option<(f64, f64)>,
    pub p_value: Option<f64>,
    pub p_method: &'static str,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    pub n_used: usize,
    pub n_unpaired: u64,
    pub metrics: Vec<MetricDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
}

/// Settings of a comparison.
#[derive(Debug, Clone)]
pub struct Options {
    pub level: f64,
    pub bootstrap: u32,
    pub seed: u64,
    pub min_effect: BTreeMap<String, f64>,
    /// Items left out of the comparison (inputs that changed between arms).
    pub excluded: BTreeSet<String>,
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

/// Compare the baseline with one candidate on the items both have.
pub fn compare(base: &Baseline<'_, '_>, b: &Prepared<'_>, o: &Options) -> Comparison {
    let a = base.prepared;
    let b_ids = b.metric_ids();
    let ids: Vec<MetricId> = a
        .metric_ids()
        .into_iter()
        .filter(|m| b_ids.contains(m))
        .collect();
    let judged_b = b.judged();
    let in_both: BTreeSet<&str> = base
        .judged
        .keys()
        .filter(|k| judged_b.contains_key(*k))
        .map(String::as_str)
        .collect();
    let paired: BTreeSet<&str> = in_both
        .iter()
        .copied()
        .filter(|k| !o.excluded.contains(*k))
        .collect();
    let union = base
        .judged
        .keys()
        .chain(judged_b.keys())
        .collect::<BTreeSet<_>>()
        .len();
    let n_unpaired = (union - in_both.len()) as u64;
    let independent = a.independent() && b.independent() && a.rule().is_some();
    let metrics = ids
        .iter()
        .map(|&id| {
            metric_delta(
                id,
                &base.values[&id],
                &values_of(b, id),
                &paired,
                independent,
                a,
                b,
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

#[allow(clippy::too_many_arguments)]
fn metric_delta(
    id: MetricId,
    va: &Filled,
    vb: &Filled,
    paired: &BTreeSet<&str>,
    independent: bool,
    a: &Prepared<'_>,
    b: &Prepared<'_>,
    o: &Options,
) -> MetricDelta {
    let sign = if id.higher_is_better() { 1.0 } else { -1.0 };
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
    let has_system_missing = a.system_missing().0 + b.system_missing().0 > 0;
    let missing_bounds = if has_system_missing && id != MetricId::Precision {
        oriented(ba, wb)
            .zip(oriented(wa, bb))
            .map(|(x, y)| (x.min(y), x.max(y)))
    } else {
        None
    };
    let min_effect = o.min_effect.get(id.name()).copied().unwrap_or(0.0);
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
    let (ci, interval, resamples, p_value, p_method) = if delta.is_none() {
        (None, "none", None, None, "none")
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
        (None, "too_few_units", None, None, "none")
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
    MetricDelta {
        metric: id.name(),
        direction: if id.higher_is_better() {
            "higher_is_better"
        } else {
            "lower_is_better"
        },
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

/// Welch's t interval for the difference of two means, with its two-sided p-value.
fn welch(a: &[f64], b: &[f64], level: f64) -> Option<((f64, f64), f64)> {
    use statrs::distribution::{ContinuousCDF, StudentsT};
    let (na, nb) = (a.len() as f64, b.len() as f64);
    if na < 2.0 || nb < 2.0 {
        return None;
    }
    let (ma, mb) = (mean(a.iter().copied())?, mean(b.iter().copied())?);
    let var = |x: &[f64], m: f64| sum(x.iter().map(|v| (v - m).powi(2))) / (x.len() - 1) as f64;
    let (sa, sb) = (var(a, ma) / na, var(b, mb) / nb);
    let se = (sa + sb).sqrt();
    if se == 0.0 {
        return None;
    }
    let df = (sa + sb).powi(2) / (sa * sa / (na - 1.0) + sb * sb / (nb - 1.0));
    let t = StudentsT::new(0.0, 1.0, df).ok()?;
    let q = t.inverse_cdf(0.5 + level / 2.0);
    let d = mb - ma;
    let p = 2.0 * (1.0 - t.cdf((d / se).abs()));
    Some(((d - q * se, d + q * se), p))
}

/// Compare two runs whose item sets need not overlap, as independent samples
/// (metrics spec §7.3): Newcombe's interval for rates of 0/1 items, Welch's t for
/// mean scores of unit-weight items, otherwise a bootstrap that resamples each arm's
/// clusters separately. Items in both arms count in both.
pub fn unpaired(a: &Prepared<'_>, b: &Prepared<'_>, o: &Options) -> Comparison {
    let b_ids = b.metric_ids();
    let ids: Vec<MetricId> = a
        .metric_ids()
        .into_iter()
        .filter(|m| b_ids.contains(m))
        .collect();
    let independent = a.independent() && b.independent();
    let z = norm_ppf(0.5 + o.level / 2.0);
    let metrics = ids
        .iter()
        .map(|&id| {
            let sign = if id.higher_is_better() { 1.0 } else { -1.0 };
            let keep = |v: BTreeMap<String, ItemValue>| -> BTreeMap<String, ItemValue> {
                v.into_iter()
                    .filter(|(k, _)| !o.excluded.contains(k))
                    .collect()
            };
            let [pa, wa, ba] = values_of(a, id).map(keep);
            let [pb, wb, bb] = values_of(b, id).map(keep);
            let all = |v: &BTreeMap<String, ItemValue>| -> Option<f64> {
                let keys: Vec<&str> = v.keys().map(String::as_str).collect();
                ratio(v, &keys)
            };
            let (base, cand) = (all(&pa), all(&pb));
            let delta = base.zip(cand).map(|(x, y)| sign * (y - x));
            let has_system_missing = a.system_missing().0 + b.system_missing().0 > 0;
            let missing_bounds = if has_system_missing && id != MetricId::Precision {
                let d = |x: &BTreeMap<String, ItemValue>, y: &BTreeMap<String, ItemValue>| {
                    all(x).zip(all(y)).map(|(x, y)| sign * (y - x))
                };
                d(&ba, &wb)
                    .zip(d(&wa, &bb))
                    .map(|(x, y)| (x.min(y), x.max(y)))
            } else {
                None
            };
            let min_effect = o.min_effect.get(id.name()).copied().unwrap_or(0.0);
            let integral = pa
                .values()
                .chain(pb.values())
                .all(|v| v.numerator.fract() == 0.0 && v.denominator.fract() == 0.0);
            let unit = |v: &BTreeMap<String, ItemValue>| {
                v.iter()
                    .all(|(k, x)| x.denominator == 1.0 && x.cluster == *k)
            };
            let points = |v: &BTreeMap<String, ItemValue>| -> Vec<f64> {
                v.values().map(|x| x.numerator).collect()
            };
            let orient = |(lo, hi): (f64, f64)| if sign > 0.0 { (lo, hi) } else { (-hi, -lo) };
            let (ci, interval, resamples, p_value, p_method) = if delta.is_none() {
                (None, "none", None, None, "none")
            } else if independent && integral && id.is_paired_proportion() {
                let k = |v: &BTreeMap<String, ItemValue>| sum(v.values().map(|x| x.numerator));
                let n = |v: &BTreeMap<String, ItemValue>| sum(v.values().map(|x| x.denominator));
                // newcombe gives p_first − p_second; candidate minus baseline here.
                let ci = newcombe(k(&pb), n(&pb), k(&pa), n(&pa), z);
                (Some(orient(ci)), "newcombe", None, None, "none")
            } else if id == MetricId::MeanScore && unit(&pa) && unit(&pb) {
                match welch(&points(&pa), &points(&pb), o.level) {
                    Some((ci, p)) => (Some(orient(ci)), "welch", None, Some(p), "welch_t"),
                    None => (None, "too_few_units", None, None, "none"),
                }
            } else {
                let clusters = |v: &BTreeMap<String, ItemValue>| -> Vec<[f64; 2]> {
                    let mut c: BTreeMap<&str, [f64; 2]> = BTreeMap::new();
                    for x in v.values() {
                        let e = c.entry(x.cluster.as_str()).or_default();
                        e[0] += x.numerator;
                        e[1] += x.denominator;
                    }
                    c.into_values().collect()
                };
                let (ca, cb) = (clusters(&pa), clusters(&pb));
                if ca.len() < bootstrap::MIN_UNITS || cb.len() < bootstrap::MIN_UNITS {
                    (None, "too_few_units", None, None, "none")
                } else {
                    let mut rng = Rng::new(o.seed);
                    let mut reps = Vec::with_capacity(o.bootstrap as usize);
                    let draw = |c: &[[f64; 2]], rng: &mut Rng| -> Option<f64> {
                        let mut t = [0.0; 2];
                        for _ in 0..c.len() {
                            let g = c[rng.below(c.len() as u64) as usize];
                            t[0] += g[0];
                            t[1] += g[1];
                        }
                        (t[1] > 0.0).then(|| t[0] / t[1])
                    };
                    for _ in 0..o.bootstrap {
                        if let (Some(x), Some(y)) = (draw(&ca, &mut rng), draw(&cb, &mut rng)) {
                            reps.push(sign * (y - x));
                        }
                    }
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
            MetricDelta {
                metric: id.name(),
                direction: if id.higher_is_better() {
                    "higher_is_better"
                } else {
                    "lower_is_better"
                },
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
        })
        .collect();
    let (ja, jb) = (a.judged(), b.judged());
    Comparison {
        n_used: ja.len() + jb.len(),
        n_unpaired: 0,
        metrics,
        transition: None,
    }
}

/// A paired difference monitored with a confidence sequence (metrics spec §7.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SequentialDelta {
    pub metric: &'static str,
    pub direction: &'static str,
    /// Paired items in the order they were added.
    pub n: usize,
    pub delta: Option<f64>,
    /// The confidence sequence after the last item; valid at every earlier look too.
    pub confidence_sequence: Option<(f64, f64)>,
    pub min_effect: f64,
    pub verdict: Verdict,
}

/// Paired differences of each metric whose item values lie in [0, 1], added in an
/// order shuffled by the seed, with the confidence sequence of their mean. An item
/// counts where both arms give it a positive denominator; its value is the ratio.
pub fn sequential(base: &Baseline<'_, '_>, b: &Prepared<'_>, o: &Options) -> Vec<SequentialDelta> {
    let a = base.prepared;
    let b_ids = b.metric_ids();
    a.metric_ids()
        .into_iter()
        .filter(|m| b_ids.contains(m))
        .filter_map(|id| {
            let sign = if id.higher_is_better() { 1.0 } else { -1.0 };
            let (pa, pb) = (&base.values[&id][0], &values_of(b, id)[0]);
            let value = |v: &ItemValue| v.numerator / v.denominator;
            let mut ds: Vec<f64> = pa
                .iter()
                .filter(|(k, x)| !o.excluded.contains(*k) && x.denominator > 0.0)
                .filter_map(|(k, x)| {
                    pb.get(k)
                        .filter(|y| y.denominator > 0.0)
                        .map(|y| (value(x), value(y)))
                })
                .map(|(x, y)| sign * (y - x))
                .collect();
            let bounded = pa
                .values()
                .chain(pb.values())
                .filter(|v| v.denominator > 0.0)
                .all(|v| (0.0..=1.0).contains(&value(v)));
            if !bounded {
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
                direction: if id.higher_is_better() {
                    "higher_is_better"
                } else {
                    "lower_is_better"
                },
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
pub fn drift(d: &MetricDelta) -> &'static str {
    match d.ci {
        None => "insufficient_data",
        Some((lo, hi)) if lo > 0.0 || hi < 0.0 => "drifted",
        Some(_) => "stable",
    }
}

/// The decision metric, Holm-adjusted p-values and best candidate of several
/// comparisons against one baseline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Multi {
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
