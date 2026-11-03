//! Paired comparison of a baseline and a candidate run on their common items
//! (metrics spec §7). Differences are oriented so that positive is better.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::bootstrap::{self, Strata};
use crate::interval::Z95;
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
    /// The difference with system-side missing values filled worst and best in both
    /// arms.
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
}

fn ratio(values: &BTreeMap<String, ItemValue>, items: &[&str]) -> Option<f64> {
    let den = sum(items
        .iter()
        .filter_map(|i| values.get(*i))
        .map(|v| v.denominator));
    (den > 0.0).then(|| {
        sum(items
            .iter()
            .filter_map(|i| values.get(*i))
            .map(|v| v.numerator))
            / den
    })
}

/// Compare two prepared runs on the items both have.
pub fn compare(a: &Prepared<'_>, b: &Prepared<'_>, o: &Options) -> Comparison {
    let ids: Vec<MetricId> = a
        .metric_ids()
        .into_iter()
        .filter(|m| b.metric_ids().contains(m))
        .collect();
    let judged_a = a.judged();
    let judged_b = b.judged();
    let paired: BTreeSet<&str> = judged_a
        .keys()
        .filter(|k| judged_b.contains_key(*k))
        .map(String::as_str)
        .collect();
    let union: BTreeSet<&str> = judged_a
        .keys()
        .chain(judged_b.keys())
        .map(String::as_str)
        .collect();
    let n_unpaired = (union.len() - paired.len()) as u64;
    let independent = a.independent() && b.independent() && a.rule().is_some();
    let metrics = ids
        .iter()
        .map(|&id| metric_delta(id, a, b, &paired, independent, o))
        .collect();
    let transition = a.positive().map(|_| {
        let mut t = Transition::default();
        for item in &paired {
            let ((va, s), (vb, _)) = (judged_a[*item], judged_b[*item]);
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

fn metric_delta(
    id: MetricId,
    a: &Prepared<'_>,
    b: &Prepared<'_>,
    paired: &BTreeSet<&str>,
    independent: bool,
    o: &Options,
) -> MetricDelta {
    let sign = if id.higher_is_better() { 1.0 } else { -1.0 };
    let (va, vb) = (
        a.item_values(id, Fill::Policy),
        b.item_values(id, Fill::Policy),
    );
    let items: Vec<&str> = paired
        .iter()
        .copied()
        .filter(|i| va.contains_key(*i) && vb.contains_key(*i))
        .collect();
    let (base, cand) = (ratio(&va, &items), ratio(&vb, &items));
    let delta = base.zip(cand).map(|(x, y)| sign * (y - x));
    let bounds = |fill: Fill| {
        let (wa, wb) = (a.item_values(id, fill), b.item_values(id, fill));
        ratio(&wa, &items)
            .zip(ratio(&wb, &items))
            .map(|(x, y)| sign * (y - x))
    };
    let has_system_missing = a.system_missing().0 + b.system_missing().0 > 0;
    let missing_bounds = if has_system_missing && id != MetricId::Precision {
        bounds(Fill::Worst)
            .zip(bounds(Fill::Best))
            .map(|(w, b)| (w.min(b), w.max(b)))
    } else {
        None
    };
    let min_effect = o.min_effect.get(id.name()).copied().unwrap_or(0.0);
    let (ci, interval, resamples, p_value, p_method) = if delta.is_none() {
        (None, "none", None, None, "none")
    } else if independent && id.is_paired_proportion() {
        // Each item counts 0 or 1 in a shared denominator: discordant pairs decide.
        let counted: Vec<&str> = items
            .iter()
            .copied()
            .filter(|i| va[*i].denominator > 0.0)
            .collect();
        let (mut only_a, mut only_b) = (0u64, 0u64);
        for i in &counted {
            match (va[*i].numerator > 0.5, vb[*i].numerator > 0.5) {
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
        let z = if (o.level - 0.95).abs() < 1e-12 {
            Z95
        } else {
            norm_ppf(0.5 + o.level / 2.0)
        };
        let ci = paired::tango(d, z).map(|(lo, hi)| if sign > 0.0 { (lo, hi) } else { (-hi, -lo) });
        (
            ci,
            "tango",
            None,
            Some(paired::mcnemar(only_a, only_b).0),
            "mcnemar_exact",
        )
    } else if items
        .iter()
        .map(|i| va[*i].cluster.as_str())
        .collect::<BTreeSet<_>>()
        .len()
        < bootstrap::MIN_UNITS
    {
        (None, "too_few_units", None, None, "none")
    } else {
        // Paired cluster bootstrap: one draw of clusters serves both arms.
        let mut clusters: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for i in &items {
            clusters.entry(va[*i].cluster.as_str()).or_default().push(i);
        }
        let groups: Vec<Vec<&str>> = clusters.into_values().collect();
        let mut rng = Rng::new(o.seed);
        let (reps, _) = bootstrap::replicates(
            groups.len(),
            None::<&Strata>,
            o.bootstrap,
            &mut rng,
            |drawn| {
                let picked: Vec<&str> = drawn
                    .iter()
                    .flat_map(|&g| groups[g].iter().copied())
                    .collect();
                ratio(&va, &picked)
                    .zip(ratio(&vb, &picked))
                    .map(|(x, y)| sign * (y - x))
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
