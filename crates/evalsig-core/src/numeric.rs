//! Numerical foundations (D-009): the seeded random number generator, fixed-order
//! summation and the distribution functions every metric uses.

use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::{Rng as _, SeedableRng};
use statrs::distribution::{Beta, ContinuousCDF, Normal};

/// The only random number generator evalsig uses: ChaCha20 from a recorded seed, so
/// results repeat on every platform.
#[derive(Debug, Clone)]
pub struct Rng {
    inner: ChaCha20Rng,
    seed: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng {
            inner: ChaCha20Rng::seed_from_u64(seed),
            seed,
        }
    }

    /// The seed to record in outputs and ledgers.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    /// Uniform in [0, 1) with 53 random bits.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in 0..n without modulo bias (Lemire's multiply-and-reject).
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below(0) has no values");
        let threshold = n.wrapping_neg() % n;
        loop {
            let m = u128::from(self.next_u64()) * u128::from(n);
            if (m as u64) >= threshold {
                return (m >> 64) as u64;
            }
        }
    }
}

/// Neumaier-compensated sum in iteration order. Callers iterate in a fixed order, so
/// the result does not depend on threads or hash maps.
pub fn sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for x in values {
        let t = s + x;
        c += if s.abs() >= x.abs() {
            (s - t) + x
        } else {
            (x - t) + s
        };
        s = t;
    }
    // With an infinite term the compensation is meaningless (it becomes inf − inf);
    // the plain sum carries the infinity or NaN.
    if s.is_finite() { s + c } else { s }
}

/// Mean in iteration order with the compensated sum; `None` when empty.
pub fn mean(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut n = 0usize;
    let total = sum(values.into_iter().inspect(|_| n += 1));
    (n > 0).then(|| total / n as f64)
}

/// Variance with divisor n − ddof (0: population, 1: sample); NaN when n ≤ ddof.
pub fn variance(xs: &[f64], ddof: usize) -> f64 {
    let m = mean(xs.iter().copied()).unwrap_or(f64::NAN);
    sum(xs.iter().map(|x| (x - m) * (x - m))) / (xs.len() as f64 - ddof as f64)
}

/// Weighted mean Σwv / Σw of (value, weight) pairs; `None` when the weights sum to 0.
pub fn weighted_mean(pairs: impl IntoIterator<Item = (f64, f64)> + Clone) -> Option<f64> {
    let w = sum(pairs.clone().into_iter().map(|(_, w)| w));
    (w > 0.0).then(|| sum(pairs.into_iter().map(|(v, w)| v * w)) / w)
}

/// Standard normal quantile Φ⁻¹(p).
pub fn norm_ppf(p: f64) -> f64 {
    standard_normal().inverse_cdf(p)
}

fn standard_normal() -> Normal {
    Normal::new(0.0, 1.0).expect("standard normal parameters are valid")
}

/// Beta(a, b) quantile. statrs's inverse of the regularized incomplete beta function
/// matches the reference values within 1e-9 (D-009), so no fallback is kept.
pub fn beta_ppf(p: f64, a: f64, b: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&p) && a > 0.0 && b > 0.0,
        "beta_ppf({p}, {a}, {b})"
    );
    Beta::new(a, b)
        .expect("shape parameters are positive")
        .inverse_cdf(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_repeats_from_its_seed() {
        let mut a = Rng::new(20261029);
        let mut b = Rng::new(20261029);
        let xs: Vec<u64> = (0..4).map(|_| a.next_u64()).collect();
        let ys: Vec<u64> = (0..4).map(|_| b.next_u64()).collect();
        assert_eq!(xs, ys);
        assert_ne!(
            xs,
            (0..4).map(|_| Rng::new(1).next_u64()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn below_is_in_range_and_roughly_uniform() {
        let mut r = Rng::new(7);
        let mut counts = [0u32; 3];
        for _ in 0..30_000 {
            counts[r.below(3) as usize] += 1;
        }
        assert!(
            counts.iter().all(|c| (9_500..10_500).contains(c)),
            "{counts:?}"
        );
        let f = r.next_f64();
        assert!((0.0..1.0).contains(&f));
    }

    #[test]
    fn compensated_sum_keeps_small_terms() {
        let xs = [1e16, 1.0, -1e16, 1.0];
        assert_eq!(sum(xs), 2.0);
        assert_eq!(sum([0.1; 10]), 1.0);
        assert_eq!(sum([f64::INFINITY, 1.0]), f64::INFINITY);
        assert_eq!(sum([1.0, f64::NEG_INFINITY]), f64::NEG_INFINITY);
        assert_eq!(sum([f64::INFINITY, 1.0]), f64::INFINITY);
        assert_eq!(sum([1.0, f64::NEG_INFINITY]), f64::NEG_INFINITY);
    }

    #[test]
    fn quantiles_invert_their_distributions() {
        assert!((norm_ppf(0.975) - 1.959_963_984_540_054).abs() < 1e-12);
        assert!((standard_normal().cdf(norm_ppf(0.3)) - 0.3).abs() < 1e-12);
        for (p, a, b) in [(0.025, 1.0, 10.0), (0.95, 4.0, 17.0), (0.5, 0.5, 0.5)] {
            let beta = Beta::new(a, b).unwrap();
            assert!((beta.cdf(beta_ppf(p, a, b)) - p).abs() < 1e-12);
        }
        assert_eq!(mean([1.0, 2.0, 6.0]), Some(3.0));
        assert_eq!(mean([]), None);
        assert_eq!(weighted_mean([(1.0, 3.0), (0.0, 1.0)]), Some(0.75));
        assert_eq!(weighted_mean([(1.0, 0.0)]), None);
    }

    /// The ChaCha20 stream and derived values for seed 20261029. The same numbers
    /// must come out on every platform; CI runs this on Linux and macOS.
    #[test]
    fn seeded_results_are_pinned() {
        let mut r = Rng::new(20261029);
        let words: Vec<u64> = (0..3).map(|_| r.next_u64()).collect();
        let draws: Vec<u64> = (0..5).map(|_| r.below(1000)).collect();
        let mean = sum((0..1000).map(|_| r.next_f64())) / 1000.0;
        assert_eq!(words, PINNED_WORDS);
        assert_eq!(draws, PINNED_DRAWS);
        assert_eq!(mean.to_bits(), PINNED_MEAN.to_bits());
    }

    const PINNED_WORDS: [u64; 3] = [
        16031999578363529886,
        3284853813119880456,
        2795450140936604411,
    ];
    const PINNED_DRAWS: [u64; 5] = [850, 168, 677, 693, 928];
    const PINNED_MEAN: f64 = 0.49170592218711534;
}
