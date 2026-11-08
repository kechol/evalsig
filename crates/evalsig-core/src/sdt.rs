//! Signal-detection metrics of binary decisions (metrics spec §2).
//!
//! Raw rates are for display. d′ and c use the log-linear correction on every cell
//! (Hautus 1995) and assume equal-variance normal distributions, which a single ROC
//! point cannot check.

use serde::Serialize;

use crate::numeric::norm_ppf;

/// Weighted counts of reference class × decision, on the item scale: an item adds its
/// item weight (1 by default), split across cells by a soft reference or by the share
/// of positive replicates. The log-linear correction (+½, +1) is in these units.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Confusion {
    /// Reference positive, judged positive.
    pub hits: f64,
    /// Reference positive, judged negative.
    pub misses: f64,
    /// Reference negative, judged positive.
    pub false_alarms: f64,
    /// Reference negative, judged negative.
    pub correct_rejections: f64,
}

/// The assumption behind d′ and c from one ROC point.
pub const ASSUMPTION: &str = "equal_variance_normal";

impl Confusion {
    fn positives(&self) -> f64 {
        self.hits + self.misses
    }

    fn negatives(&self) -> f64 {
        self.false_alarms + self.correct_rejections
    }

    /// Log-linear corrected H′ = (hits + ½) / (positives + 1).
    pub fn corrected_hit_rate(&self) -> f64 {
        (self.hits + 0.5) / (self.positives() + 1.0)
    }

    /// Log-linear corrected F′ = (false alarms + ½) / (negatives + 1).
    pub fn corrected_false_alarm_rate(&self) -> f64 {
        (self.false_alarms + 0.5) / (self.negatives() + 1.0)
    }

    /// d′ = z(H′) − z(F′) and c = −(z(H′) + z(F′)) / 2; a negative c means liberal.
    pub fn dprime_and_criterion(&self) -> (f64, f64) {
        let (zh, zf) = (
            norm_ppf(self.corrected_hit_rate()),
            norm_ppf(self.corrected_false_alarm_rate()),
        );
        (zh - zf, -(zh + zf) / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn flipping_the_positive_class_flips_dprime(
            h in 0u32..200, m in 0u32..200, f in 0u32..200, r in 0u32..200,
        ) {
            let (h, m, f, r) = (f64::from(h), f64::from(m), f64::from(f), f64::from(r));
            let c = Confusion { hits: h, misses: m, false_alarms: f, correct_rejections: r };
            let flipped = Confusion { hits: f, misses: r, false_alarms: h, correct_rejections: m };
            prop_assert!((c.dprime_and_criterion().0 + flipped.dprime_and_criterion().0).abs() < 1e-12);
        }
    }
}
