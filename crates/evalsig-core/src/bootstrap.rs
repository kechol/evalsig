//! The cluster bootstrap (metrics spec §8).
//!
//! Only the top level is resampled: clusters when items have them, otherwise items.
//! Replicates, raters and runs stay fixed inside each unit, because the top-level
//! resample already carries their variability; resampling them too would count it
//! twice. Paired comparisons draw the same units for both arms.

use crate::numeric::Rng;

/// Default number of resamples.
pub const DEFAULT_B: u32 = 2000;

/// Fewest resampling units (clusters, else items) for a bootstrap interval. With
/// four units there are only 35 distinct resamples, too few for a percentile
/// interval to cover at 95%; below this the interval is not reported.
pub const MIN_UNITS: usize = 5;

/// Resamples a gate needs at one-sided level `alpha`: B ≥ 50 / α.
pub fn required_resamples(alpha: f64) -> u32 {
    (50.0 / alpha).ceil() as u32
}

/// Units grouped by stratum, built once and reused by every resample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strata {
    groups: Vec<Vec<usize>>,
    n_units: usize,
}

impl Strata {
    /// `of_unit[i]` is the stratum of unit i.
    pub fn new(of_unit: &[usize]) -> Self {
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for (unit, &s) in of_unit.iter().enumerate() {
            if groups.len() <= s {
                groups.resize_with(s + 1, Vec::new);
            }
            groups[s].push(unit);
        }
        Strata {
            groups,
            n_units: of_unit.len(),
        }
    }
}

/// Draw one resample into `out`: indices into the units, with replacement. With
/// strata, each stratum keeps its own size (used only for metrics conditioned on the
/// reference class, such as H, F and d′).
pub fn resample_into(out: &mut Vec<usize>, n_units: usize, strata: Option<&Strata>, rng: &mut Rng) {
    out.clear();
    match strata {
        None => out.extend((0..n_units).map(|_| rng.below(n_units as u64) as usize)),
        Some(st) => {
            assert_eq!(st.n_units, n_units, "one stratum per unit");
            for g in &st.groups {
                let len = g.len() as u64;
                out.extend((0..g.len()).map(|_| g[rng.below(len) as usize]));
            }
        }
    }
}

/// The statistic over `b` resamples. A resample where the statistic is undefined
/// (for example no positives drawn) yields `None` and is dropped; the count of
/// dropped resamples is returned so callers can report it.
pub fn replicates(
    n_units: usize,
    strata: Option<&Strata>,
    b: u32,
    rng: &mut Rng,
    mut statistic: impl FnMut(&[usize]) -> Option<f64>,
) -> (Vec<f64>, u32) {
    let mut out = Vec::with_capacity(b as usize);
    let mut dropped = 0;
    let mut idx = Vec::with_capacity(n_units);
    for _ in 0..b {
        resample_into(&mut idx, n_units, strata, rng);
        match statistic(&idx) {
            Some(v) if v.is_finite() => out.push(v),
            _ => dropped += 1,
        }
    }
    (out, dropped)
}

/// Quantile by linear interpolation between order statistics (the common "type 7").
pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    assert!(!sorted.is_empty(), "quantile of nothing");
    let h = (sorted.len() - 1) as f64 * q.clamp(0.0, 1.0);
    let (lo, hi) = (h.floor() as usize, h.ceil() as usize);
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

/// Two-sided percentile interval at confidence `level`.
pub fn percentile_interval(mut replicates: Vec<f64>, level: f64) -> Option<(f64, f64)> {
    if replicates.is_empty() {
        return None;
    }
    replicates.sort_by(f64::total_cmp);
    let a = (1.0 - level) / 2.0;
    Some((quantile(&replicates, a), quantile(&replicates, 1.0 - a)))
}

/// Two-sided p-value of an observed difference: the share of resampled differences,
/// shifted to centre on 0, at least as far from 0 as the observed one, with the +1
/// correction so it never reaches 0 (a bootstrap cannot resolve below 1/(B + 1)).
/// Callers report dropped resamples next to it.
pub fn p_value(replicates: &[f64], observed: f64) -> Option<f64> {
    if replicates.is_empty() {
        return None;
    }
    let extreme = replicates
        .iter()
        .filter(|d| (*d - observed).abs() >= observed.abs())
        .count();
    Some((extreme + 1) as f64 / (replicates.len() + 1) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 40 clusters of 5 items; within a cluster every item has the same outcome, so
    /// the effective sample is 40, not 200.
    fn clustered() -> (Vec<Vec<f64>>, Vec<f64>) {
        let clusters: Vec<Vec<f64>> = (0..40)
            .map(|c| vec![f64::from(u8::from(c % 3 == 0)); 5])
            .collect();
        let flat = clusters.iter().flatten().copied().collect();
        (clusters, flat)
    }

    fn mean_of(units: &[Vec<f64>], idx: &[usize]) -> Option<f64> {
        let (s, n) = idx.iter().fold((0.0, 0usize), |(s, n), &i| {
            (s + units[i].iter().sum::<f64>(), n + units[i].len())
        });
        (n > 0).then(|| s / n as f64)
    }

    #[test]
    fn same_seed_same_interval_and_pinned_values() {
        let (clusters, _) = clustered();
        let run = || {
            let (reps, dropped) =
                replicates(clusters.len(), None, 2000, &mut Rng::new(20261029), |i| {
                    mean_of(&clusters, i)
                });
            assert_eq!(dropped, 0);
            let total = crate::numeric::sum(reps.iter().copied());
            (percentile_interval(reps, 0.95).unwrap(), total)
        };
        let (a, total) = run();
        assert_eq!((a, total), run());
        assert_eq!(a, (0.2, 0.5));
        assert_eq!(total.to_bits(), PINNED_TOTAL.to_bits(), "{total:?}");
    }

    /// Sum of the 2000 replicate means for seed 20261029, the same on every platform.
    const PINNED_TOTAL: f64 = 700.3;

    #[test]
    fn clusters_widen_the_interval() {
        let (clusters, flat) = clustered();
        let items: Vec<Vec<f64>> = flat.iter().map(|&x| vec![x]).collect();
        let width = |units: &Vec<Vec<f64>>| {
            let (reps, _) = replicates(units.len(), None, 2000, &mut Rng::new(1), |i| {
                mean_of(units, i)
            });
            let (lo, hi) = percentile_interval(reps, 0.95).unwrap();
            hi - lo
        };
        assert!(
            width(&clusters) > 1.8 * width(&items),
            "{} vs {}",
            width(&clusters),
            width(&items)
        );
    }

    #[test]
    fn strata_keep_their_sizes() {
        let strata = [0, 0, 0, 1, 1];
        let st = Strata::new(&strata);
        let mut rng = Rng::new(3);
        let mut idx = Vec::new();
        for _ in 0..100 {
            resample_into(&mut idx, 5, Some(&st), &mut rng);
            assert_eq!(idx.iter().filter(|&&i| strata[i] == 0).count(), 3);
            assert_eq!(idx.len(), 5);
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(required_resamples(0.0025), 20000);
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.5);
        assert_eq!(p_value(&[0.0, 0.2, 0.5], 0.2), Some(3.0 / 4.0));
        assert_eq!(p_value(&[4.0, 5.5], 5.0), Some(1.0 / 3.0));
        assert_eq!(percentile_interval(vec![], 0.95), None);
    }
}
