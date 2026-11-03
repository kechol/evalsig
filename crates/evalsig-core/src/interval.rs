//! Intervals for rates and how to choose one (metrics spec §1, §1.1).
//!
//! Counts are `f64` so that effective sizes (clusters, Kish n_eff) can stand in for
//! item counts, as the specification requires when items are not independent.

use serde::Serialize;

use crate::config::IntervalMethod;
use crate::numeric::{beta_ppf, norm_ppf};

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
    wilson(k, n, norm_ppf(0.5 + level / 2.0))
}

/// One-sided Wilson upper bound at confidence `level`.
pub fn wilson_upper(k: f64, n: f64, level: f64) -> f64 {
    wilson(k, n, norm_ppf(level)).1
}

/// Clopper-Pearson lower bound with one-sided confidence `level`.
pub fn clopper_pearson_lower(k: f64, n: f64, level: f64) -> f64 {
    if k <= 0.0 {
        0.0
    } else {
        beta_ppf(1.0 - level, k, n - k + 1.0)
    }
}

/// Clopper-Pearson upper bound with one-sided confidence `level`; 1 when k = n.
pub fn clopper_pearson_upper(k: f64, n: f64, level: f64) -> f64 {
    if k >= n {
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
    let (pa, pb) = (ka / na.max(1.0), kb / nb.max(1.0));
    let (la, ua) = wilson(ka, na, z);
    let (lb, ub) = wilson(kb, nb, z);
    let d = pa - pb;
    (
        d - ((pa - la).powi(2) + (ub - pb).powi(2)).sqrt(),
        d + ((ua - pa).powi(2) + (pb - lb).powi(2)).sqrt(),
    )
}

/// The smallest number of items with zero events whose one-sided upper bound at
/// `level` falls below `bound`, for the exact method and for Wilson.
pub fn zero_event_items(bound: f64, level: f64) -> (u64, u64) {
    let smallest = |upper: &dyn Fn(f64) -> f64| {
        let mut n = 1u64;
        while upper(n as f64) >= bound {
            n += 1;
        }
        n
    };
    (
        smallest(&|n| clopper_pearson_upper(0.0, n, level)),
        smallest(&|n| wilson_upper(0.0, n, level)),
    )
}

/// What an interval is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Shown in reports (two-sided).
    Display,
    /// A gate's one-sided bound.
    Gate,
}

/// The shape of the estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estimate {
    /// One rate: accuracy, H, F, disagreement, missing rate, share at or above t.
    Rate,
    /// A paired difference of two rates.
    PairedRateDifference,
    /// Mean-score differences, d′, expected cost, κ, win rates.
    Other,
    /// AUC against a binary reference.
    AucBinary,
    /// AUC against a soft or latent reference.
    AucSoft,
}

/// The method that computes an interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Wilson,
    ClopperPearson,
    Jeffreys,
    Bootstrap,
    Tango,
    DeLong,
}

/// Choose the interval method (metrics spec §1.1). `dependent` means replicates,
/// clusters or weights are present, so counts become effective sizes or the
/// bootstrap is used. `events` is the event count of a rate-type gate condition.
pub fn select(
    estimate: Estimate,
    dependent: bool,
    purpose: Purpose,
    configured: IntervalMethod,
    events: Option<u64>,
) -> Result<Method, String> {
    let auto = match (estimate, dependent, purpose) {
        (Estimate::Rate, false, Purpose::Display) => Method::Wilson,
        (Estimate::Rate, _, Purpose::Gate) => Method::ClopperPearson,
        (Estimate::Rate, true, Purpose::Display) => Method::Bootstrap,
        (Estimate::PairedRateDifference, false, _) => Method::Tango,
        (Estimate::PairedRateDifference, true, _) => Method::Bootstrap,
        (Estimate::Other, _, _) => Method::Bootstrap,
        (Estimate::AucBinary, _, _) => Method::DeLong,
        (Estimate::AucSoft, _, _) => Method::Bootstrap,
    };
    let chosen = match configured {
        IntervalMethod::Auto => auto,
        IntervalMethod::Wilson => Method::Wilson,
        IntervalMethod::ClopperPearson => Method::ClopperPearson,
        IntervalMethod::Jeffreys => Method::Jeffreys,
        IntervalMethod::Bootstrap => Method::Bootstrap,
    };
    let rate_method = matches!(
        chosen,
        Method::Wilson | Method::ClopperPearson | Method::Jeffreys
    );
    if rate_method && estimate != Estimate::Rate {
        return Err(format!("{chosen:?} applies to single rates only"));
    }
    if purpose == Purpose::Gate
        && estimate == Estimate::Rate
        && chosen == Method::Bootstrap
        && events.is_some_and(|e| e <= 5)
    {
        return Err(
            "bootstrap bounds degenerate with 5 or fewer events; use the exact method".into(),
        );
    }
    Ok(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_follows_the_table() {
        use Estimate::*;
        use Purpose::*;
        let auto = |e, d, p| select(e, d, p, IntervalMethod::Auto, None).unwrap();
        assert_eq!(auto(Rate, false, Display), Method::Wilson);
        assert_eq!(auto(Rate, false, Gate), Method::ClopperPearson);
        assert_eq!(auto(Rate, true, Gate), Method::ClopperPearson);
        assert_eq!(auto(Rate, true, Display), Method::Bootstrap);
        assert_eq!(auto(PairedRateDifference, false, Display), Method::Tango);
        assert_eq!(auto(PairedRateDifference, true, Display), Method::Bootstrap);
        assert_eq!(auto(Other, false, Display), Method::Bootstrap);
        assert_eq!(auto(AucBinary, false, Display), Method::DeLong);
        assert_eq!(auto(AucSoft, false, Display), Method::Bootstrap);
        assert_eq!(
            select(Rate, false, Display, IntervalMethod::Jeffreys, None),
            Ok(Method::Jeffreys)
        );
        assert!(select(Other, false, Display, IntervalMethod::Wilson, None).is_err());
        assert!(select(Rate, false, Gate, IntervalMethod::Bootstrap, Some(5)).is_err());
        assert_eq!(
            select(Rate, false, Gate, IntervalMethod::Bootstrap, Some(6)),
            Ok(Method::Bootstrap)
        );
    }

    #[test]
    fn effective_sizes_are_accepted() {
        let (lo, hi) = clopper_pearson(3.5, 20.25, 0.95);
        assert!(0.0 < lo && lo < 3.5 / 20.25 && 3.5 / 20.25 < hi && hi < 1.0);
    }
}
