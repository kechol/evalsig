//! Paired comparison of a baseline and candidate runs on their common items (metrics
//! spec §7). Differences are oriented so that positive is better.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bootstrap::{self, Strata};
use crate::numeric::{Rng, norm_ppf, sum};
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
