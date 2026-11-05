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
    /// Add a (reference positive, judged positive) pair with its weight.
    pub fn add(&mut self, reference: bool, judged: bool, weight: f64) {
        match (reference, judged) {
            (true, true) => self.hits += weight,
            (true, false) => self.misses += weight,
            (false, true) => self.false_alarms += weight,
            (false, false) => self.correct_rejections += weight,
        }
    }

    /// Add an item whose reference is positive with share `reference` (soft) and whose
    /// judgments are positive with share `judged`.
    pub fn add_shares(&mut self, reference: f64, judged: f64, weight: f64) {
        self.hits += weight * reference * judged;
        self.misses += weight * reference * (1.0 - judged);
        self.false_alarms += weight * (1.0 - reference) * judged;
        self.correct_rejections += weight * (1.0 - reference) * (1.0 - judged);
    }

    /// Accumulate a confusion table from (reference positive, judged positive, weight)
    /// triples.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (bool, bool, f64)>) -> Self {
        let mut c = Confusion::default();
        for (r, j, w) in pairs {
            c.add(r, j, w);
        }
        c
    }

    /// Total weight of reference positives (hits + misses).
    pub fn positives(&self) -> f64 {
        self.hits + self.misses
    }

    /// Total weight of reference negatives (false alarms + correct rejections).
    pub fn negatives(&self) -> f64 {
        self.false_alarms + self.correct_rejections
    }

    /// Total weight of all items.
    pub fn total(&self) -> f64 {
        self.positives() + self.negatives()
    }

    fn ratio(a: f64, b: f64) -> Option<f64> {
        (b > 0.0).then(|| a / b)
    }

    /// Share of weight on which the judgment equals the reference; `None` for an empty
    /// table.
    pub fn accuracy(&self) -> Option<f64> {
        Self::ratio(self.hits + self.correct_rejections, self.total())
    }

    /// Raw hit rate H.
    pub fn hit_rate(&self) -> Option<f64> {
        Self::ratio(self.hits, self.positives())
    }

    /// Raw false alarm rate F.
    pub fn false_alarm_rate(&self) -> Option<f64> {
        Self::ratio(self.false_alarms, self.negatives())
    }

    /// Precision in the test set.
    pub fn precision(&self) -> Option<f64> {
        Self::ratio(self.hits, self.hits + self.false_alarms)
    }

    /// Precision at a production prevalence π: πH / (πH + (1 − π)F).
    pub fn precision_at(&self, prevalence: f64) -> Option<f64> {
        let (h, f) = (self.hit_rate()?, self.false_alarm_rate()?);
        Self::ratio(prevalence * h, prevalence * h + (1.0 - prevalence) * f)
    }

    /// Log-linear corrected H′ = (hits + ½) / (positives + 1).
    pub fn corrected_hit_rate(&self) -> f64 {
        (self.hits + 0.5) / (self.positives() + 1.0)
    }

    /// Log-linear corrected F′ = (false alarms + ½) / (negatives + 1).
    pub fn corrected_false_alarm_rate(&self) -> f64 {
        (self.false_alarms + 0.5) / (self.negatives() + 1.0)
    }

    /// (z(H′), z(F′)), shared by d′ and c.
    fn z_rates(&self) -> (f64, f64) {
        (
            norm_ppf(self.corrected_hit_rate()),
            norm_ppf(self.corrected_false_alarm_rate()),
        )
    }

    /// Sensitivity index d′ = z(H′) − z(F′).
    pub fn dprime(&self) -> f64 {
        let (zh, zf) = self.z_rates();
        zh - zf
    }

    /// Criterion c = −(z(H′) + z(F′)) / 2; negative means liberal.
    pub fn criterion(&self) -> f64 {
        let (zh, zf) = self.z_rates();
        -(zh + zf) / 2.0
    }

    /// d′ and c together, computing each quantile once (for resampling loops).
    pub fn dprime_and_criterion(&self) -> (f64, f64) {
        let (zh, zf) = self.z_rates();
        (zh - zf, -(zh + zf) / 2.0)
    }

    /// The same decisions read against the opposite positive class.
    pub fn with_reference_flipped(&self) -> Confusion {
        Confusion {
            hits: self.false_alarms,
            misses: self.correct_rejections,
            false_alarms: self.hits,
            correct_rejections: self.misses,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rates_and_precision() {
        let c = Confusion {
            hits: 40.0,
            misses: 10.0,
            false_alarms: 5.0,
            correct_rejections: 45.0,
        };
        assert_eq!(c.hit_rate(), Some(0.8));
        assert_eq!(c.false_alarm_rate(), Some(0.1));
        assert_eq!(c.accuracy(), Some(0.85));
        assert_eq!(c.precision(), Some(40.0 / 45.0));
        let p = c.precision_at(0.12).unwrap();
        assert!((p - 0.12 * 0.8 / (0.12 * 0.8 + 0.88 * 0.1)).abs() < 1e-15);
        assert_eq!(Confusion::default().hit_rate(), None);
    }

    proptest! {
        #[test]
        fn flipping_the_positive_class_flips_dprime(
            h in 0u32..200, m in 0u32..200, f in 0u32..200, r in 0u32..200,
        ) {
            let c = Confusion {
                hits: f64::from(h),
                misses: f64::from(m),
                false_alarms: f64::from(f),
                correct_rejections: f64::from(r),
            };
            let flipped = c.with_reference_flipped();
            prop_assert!((c.dprime() + flipped.dprime()).abs() < 1e-12);
        }

        #[test]
        fn raw_rates_do_not_change_when_counts_scale(
            h in 1u32..100, m in 1u32..100, f in 1u32..100, r in 1u32..100, k in 2u32..10,
        ) {
            let c = Confusion::from_pairs([
                (true, true, f64::from(h)), (true, false, f64::from(m)),
                (false, true, f64::from(f)), (false, false, f64::from(r)),
            ]);
            let s = f64::from(k);
            let scaled = Confusion {
                hits: c.hits * s, misses: c.misses * s,
                false_alarms: c.false_alarms * s, correct_rejections: c.correct_rejections * s,
            };
            prop_assert!((c.hit_rate().unwrap() - scaled.hit_rate().unwrap()).abs() < 1e-12);
            prop_assert!((c.false_alarm_rate().unwrap() - scaled.false_alarm_rate().unwrap()).abs() < 1e-12);
        }
    }
}
