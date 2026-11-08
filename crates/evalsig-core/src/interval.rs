//! Intervals for rates (metrics spec §1).
//!
//! Counts are `f64` so that effective sizes (clusters, Kish n_eff) can stand in for
//! item counts, as the specification requires when items are not independent.

use crate::numeric::{beta_ppf, norm_ppf, two_sided_z};

/// z for a two-sided 95% interval, Φ⁻¹(0.975).
pub const Z95: f64 = 1.959_963_984_540_054;

/// Wilson score interval for k successes out of n, at the given z.
pub fn wilson(k: f64, n: f64, z: f64) -> (f64, f64) {
    if n <= 0.0 {
        return (0.0, 1.0);
    }
    let p = k / n;
    let z2 = z * z;
    let d = 1.0 + z2 / n;
    let c = (p + z2 / (2.0 * n)) / d;
    let h = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

/// Two-sided Wilson interval at confidence `level`.
pub fn wilson_two_sided(k: f64, n: f64, level: f64) -> (f64, f64) {
    wilson(k, n, two_sided_z(level))
}

/// One-sided Wilson upper bound at confidence `level`.
pub fn wilson_upper(k: f64, n: f64, level: f64) -> f64 {
    wilson(k, n, norm_ppf(level)).1
}

// Every interval function returns the vacuous [0, 1] when n = 0, so callers decide
// once, when reporting, how an empty denominator is shown.

/// Clopper-Pearson lower bound with one-sided confidence `level`.
pub fn clopper_pearson_lower(k: f64, n: f64, level: f64) -> f64 {
    if k <= 0.0 || n <= 0.0 {
        0.0
    } else {
        beta_ppf(1.0 - level, k, n - k + 1.0)
    }
}

/// Clopper-Pearson upper bound with one-sided confidence `level`; 1 when k = n.
pub fn clopper_pearson_upper(k: f64, n: f64, level: f64) -> f64 {
    if k >= n || n <= 0.0 {
        1.0
    } else {
        beta_ppf(level, k + 1.0, n - k)
    }
}

/// Two-sided Clopper-Pearson (exact) interval at confidence `level`.
pub fn clopper_pearson(k: f64, n: f64, level: f64) -> (f64, f64) {
    let one_sided = 0.5 + level / 2.0;
    (
        clopper_pearson_lower(k, n, one_sided),
        clopper_pearson_upper(k, n, one_sided),
    )
}

/// Two-sided Jeffreys interval at confidence `level`.
pub fn jeffreys(k: f64, n: f64, level: f64) -> (f64, f64) {
    if n <= 0.0 {
        return (0.0, 1.0);
    }
    let a = (1.0 - level) / 2.0;
    let lo = if k <= 0.0 {
        0.0
    } else {
        beta_ppf(a, k + 0.5, n - k + 0.5)
    };
    let hi = if k >= n {
        1.0
    } else {
        beta_ppf(1.0 - a, k + 0.5, n - k + 0.5)
    };
    (lo, hi)
}

/// Newcombe's hybrid score interval (method 10) for p_a − p_b of two independent
/// proportions, from Wilson intervals at the given z.
pub fn newcombe(ka: f64, na: f64, kb: f64, nb: f64, z: f64) -> (f64, f64) {
    if na <= 0.0 || nb <= 0.0 {
        return (-1.0, 1.0);
    }
    let (pa, pb) = (ka / na, kb / nb);
    let (la, ua) = wilson(ka, na, z);
    let (lb, ub) = wilson(kb, nb, z);
    let d = pa - pb;
    (
        d - ((pa - la).powi(2) + (ub - pb).powi(2)).sqrt(),
        d + ((ua - pa).powi(2) + (pb - lb).powi(2)).sqrt(),
    )
}

/// The smallest number of items with zero events whose one-sided upper bound at
/// `level` falls below `bound`, for the exact method and for Wilson. `None` unless
/// both lie strictly between 0 and 1.
pub fn zero_event_items(bound: f64, level: f64) -> Option<(u64, u64)> {
    if !(bound > 0.0 && bound < 1.0 && level > 0.0 && level < 1.0) {
        return None;
    }
    let smallest = |upper: fn(f64, f64, f64) -> f64| {
        let mut n = 1u64;
        while upper(0.0, n as f64, level) >= bound {
            n += 1;
        }
        n
    };
    Some((smallest(clopper_pearson_upper), smallest(wilson_upper)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_denominators_are_vacuous() {
        assert_eq!(clopper_pearson(0.0, 0.0, 0.95), (0.0, 1.0));
        assert_eq!(jeffreys(0.0, 0.0, 0.95), (0.0, 1.0));
        assert_eq!(wilson_two_sided(0.0, 0.0, 0.95), (0.0, 1.0));
    }

    #[test]
    fn effective_sizes_are_accepted() {
        let (lo, hi) = clopper_pearson(3.5, 20.25, 0.95);
        assert!(0.0 < lo && lo < 3.5 / 20.25 && 3.5 / 20.25 < hi && hi < 1.0);
    }

    proptest::proptest! {
        /// Scaling every count by s keeps the raw rate and narrows the interval.
        #[test]
        fn intervals_narrow_when_counts_scale(k in 1u32..50, extra in 1u32..50, s in 2u32..10) {
            let (k, n, s) = (f64::from(k), f64::from(k + extra), f64::from(s));
            let width = |(lo, hi): (f64, f64)| hi - lo;
            proptest::prop_assert!((k / n - (k * s) / (n * s)).abs() < 1e-15);
            proptest::prop_assert!(width(wilson(k * s, n * s, Z95)) < width(wilson(k, n, Z95)));
            proptest::prop_assert!(
                width(clopper_pearson(k * s, n * s, 0.95)) < width(clopper_pearson(k, n, 0.95))
            );
        }
    }
}
