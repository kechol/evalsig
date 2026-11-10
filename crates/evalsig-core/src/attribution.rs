//! Exclusive attribution of errors to causes (metrics spec §13, D-048): each error goes
//! to its first cause in the configured order, so overlapping causes are counted once.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::bootstrap;
use crate::numeric::{Rng, sum};
use crate::report::{Fill, ItemValue, MetricId, Prepared};

/// The cause of errors no attribution names.
pub const UNATTRIBUTED: &str = "unattributed";

/// Which errors are decomposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Errors {
    /// Positives judged negative.
    Misses,
    /// Negatives judged positive.
    FalseAlarms,
}

/// The errors of a run split by cause, with the rates left if causes were removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decomposition {
    /// Which errors.
    pub errors: Errors,
    /// The miss rate or false-alarm rate.
    pub rate: f64,
    /// Its cluster-bootstrap interval.
    pub ci: Option<(f64, f64)>,
    /// Weighted number of errors.
    pub n_errors: f64,
    /// Each cause's share of the errors, in the order causes are assigned.
    pub causes: Vec<CauseShare>,
    /// The rate left if the errors of each requested set of causes were removed.
    pub ceilings: Vec<Ceiling>,
}

/// One cause's errors.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CauseShare {
    /// The cause, as the raters wrote it, or `unattributed`.
    pub cause: String,
    /// Weighted number of errors assigned to it.
    pub errors: f64,
    /// Its share of all errors.
    pub share: f64,
    /// Cluster-bootstrap interval of the share.
    pub ci: Option<(f64, f64)>,
}

/// The rate if every error of these causes were fixed and none added: an optimistic bound.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ceiling {
    /// The causes removed.
    pub removed: Vec<String>,
    /// The rate left.
    pub rate: f64,
    /// Cluster-bootstrap interval of that rate.
    pub ci: Option<(f64, f64)>,
}

/// One cluster's (or resample's) denominator and errors per cause.
struct Totals {
    den: f64,
    errs: Vec<f64>,
}

impl Totals {
    fn zero(causes: usize) -> Self {
        Totals {
            den: 0.0,
            errs: vec![0.0; causes],
        }
    }
}

/// The cause an error goes to: the first of `causes` in `order`, then the others by name.
fn assigned<'a>(causes: &'a BTreeSet<String>, order: &[String]) -> &'a str {
    order
        .iter()
        .find_map(|o| causes.get(o))
        .or_else(|| causes.first())
        .map_or(UNATTRIBUTED, String::as_str)
}

/// Decompose a run's misses or false alarms by the causes attributed to each item.
/// `None` without decisions and a reference.
pub fn decompose(
    p: &Prepared<'_>,
    errors: Errors,
    causes: &BTreeMap<String, BTreeSet<String>>,
    order: &[String],
    ceilings: &[Vec<String>],
) -> Option<Decomposition> {
    p.positive()?;
    let id = match errors {
        Errors::Misses => MetricId::HitRate,
        Errors::FalseAlarms => MetricId::FalseAlarmRate,
    };
    let values = p.item_values(id, Fill::Policy);
    // Weighted counts: a difference of sums can leave rounding residue on an item
    // with no error.
    let error = |v: &ItemValue| {
        let e = match errors {
            Errors::Misses => v.denominator - v.numerator,
            Errors::FalseAlarms => v.numerator,
        };
        if e > 1e-12 { e } else { 0.0 }
    };
    let none = BTreeSet::new();
    let cause_of = |item: &str| assigned(causes.get(item).unwrap_or(&none), order);
    // Causes in assignment order: the configured ones, then the others by name.
    let mut names: Vec<String> = Vec::new();
    for o in order {
        if !names.contains(o) {
            names.push(o.clone());
        }
    }
    let mut extra: BTreeSet<&str> = BTreeSet::new();
    for (item, v) in &values {
        let c = cause_of(item);
        if error(v) > 0.0 && !names.iter().any(|n| n == c) {
            extra.insert(c);
        }
    }
    let unattributed = extra.remove(UNATTRIBUTED);
    names.extend(extra.into_iter().map(str::to_owned));
    if unattributed {
        names.push(UNATTRIBUTED.to_owned());
    }
    let index: BTreeMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let mut clusters: BTreeMap<&str, Totals> = BTreeMap::new();
    for (item, v) in &values {
        let t = clusters
            .entry(v.cluster.as_str())
            .or_insert_with(|| Totals::zero(names.len()));
        t.den += v.denominator;
        if let Some(k) = index.get(cause_of(item)) {
            t.errs[*k] += error(v);
        }
    }
    let totals: Vec<Totals> = clusters.into_values().collect();
    let sets: Vec<Vec<usize>> = ceilings
        .iter()
        .map(|s| {
            s.iter()
                .filter_map(|c| index.get(c.as_str()).copied())
                .collect()
        })
        .collect();
    // In order: the rate, each cause's share, each ceiling.
    let stats =
        |t: &Totals| -> Vec<Option<f64>> {
            let all = sum(t.errs.iter().copied());
            std::iter::once((t.den > 0.0).then(|| all / t.den))
                .chain(t.errs.iter().map(|e| (all > 0.0).then(|| e / all)))
                .chain(sets.iter().map(|s| {
                    (t.den > 0.0).then(|| (all - sum(s.iter().map(|k| t.errs[*k]))) / t.den)
                }))
                .collect()
        };
    let add = |drawn: &mut dyn Iterator<Item = &Totals>| {
        let mut t = Totals::zero(names.len());
        for c in drawn {
            t.den += c.den;
            t.errs.iter_mut().zip(&c.errs).for_each(|(x, y)| *x += y);
        }
        t
    };
    let all = add(&mut totals.iter());
    let point = stats(&all);
    let rate = point[0]?;
    let s = p.settings();
    // Clusters with items in the denominator are the resampling units that count.
    let carrying = totals.iter().filter(|t| t.den > 0.0).count();
    let cis: Vec<Option<(f64, f64)>> = if carrying >= bootstrap::MIN_UNITS {
        let mut rng = Rng::new(s.seed);
        bootstrap::replicates_many(
            totals.len(),
            None,
            s.bootstrap,
            &mut rng,
            point.len(),
            |d| stats(&add(&mut d.iter().map(|&c| &totals[c]))),
        )
        .into_iter()
        .map(|r| bootstrap::percentile_interval(r, s.level))
        .collect()
    } else {
        vec![None; point.len()]
    };
    let (shares, ceiling_rates) = point[1..].split_at(names.len());
    let (share_cis, ceiling_cis) = cis[1..].split_at(names.len());
    let causes = names
        .iter()
        .zip(&all.errs)
        .zip(shares.iter().zip(share_cis))
        .map(|((cause, errors), (share, ci))| CauseShare {
            cause: cause.clone(),
            errors: *errors,
            share: share.unwrap_or(0.0),
            ci: *ci,
        })
        .collect();
    let ceilings = ceilings
        .iter()
        .zip(ceiling_rates.iter().zip(ceiling_cis))
        .map(|(removed, (rate, ci))| Ceiling {
            removed: removed.clone(),
            rate: rate.unwrap_or(0.0),
            ci: *ci,
        })
        .collect();
    Some(Decomposition {
        errors,
        rate,
        ci: cis[0],
        n_errors: sum(all.errs.iter().copied()),
        causes,
        ceilings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn an_error_goes_to_its_first_cause_in_order_then_by_name() {
        let order = ["b".to_owned(), "a".to_owned()];
        assert_eq!(assigned(&set(&["a", "b"]), &order), "b");
        assert_eq!(assigned(&set(&["z", "a"]), &order), "a");
        assert_eq!(assigned(&set(&["z", "y"]), &order), "y");
        assert_eq!(assigned(&set(&[]), &order), UNATTRIBUTED);
        assert_eq!(assigned(&set(&["x"]), &[]), "x");
    }
}
