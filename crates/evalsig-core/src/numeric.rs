//! Numerical foundations (D-036): the seeded random number generator, fixed-order
//! summation and the distribution functions every metric uses.

use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::{Rng as _, SeedableRng};
use statrs::distribution::{Beta, Continuous, ContinuousCDF, Normal};

/// The only random number generator evalsig uses: ChaCha20 from a recorded seed, so
/// results repeat on every platform.
#[derive(Debug, Clone)]
pub struct Rng {
    inner: ChaCha20Rng,
    seed: u64,
}

impl Rng {
    /// Seed a generator; the same seed gives the same sequence on every platform.
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

    /// The next 64 random bits.
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

/// Neumaier-compensated running sum. After k `add`s, `total` equals [`sum`] of those k
/// values, so a total can be read at every step without summing again from the start.
#[derive(Debug, Clone, Copy, Default)]
pub struct Running {
    s: f64,
    c: f64,
}

impl Running {
    /// Add one term.
    pub fn add(&mut self, x: f64) {
        let t = self.s + x;
        self.c += if self.s.abs() >= x.abs() {
            (self.s - t) + x
        } else {
            (x - t) + self.s
        };
        self.s = t;
    }

    /// The sum of the terms added so far.
    pub fn total(&self) -> f64 {
        // With an infinite term the compensation is meaningless (it becomes inf − inf);
        // the plain sum carries the infinity or NaN.
        if self.s.is_finite() {
            self.s + self.c
        } else {
            self.s
        }
    }
}

/// A number rounded to 10 decimal places (metrics spec §0), as outputs carry it, so that
/// `verify` compares values that do not depend on the last bits. The decimal string
/// decides ties, as in the written output.
pub fn round_output(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let rounded: f64 = format!("{x:.10}").parse().expect("formatted float parses");
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// Neumaier-compensated sum in iteration order. Callers iterate in a fixed order, so
/// the result does not depend on threads or hash maps.
pub fn sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut acc = Running::default();
    for x in values {
        acc.add(x);
    }
    acc.total()
}

/// Mean in iteration order with the compensated sum; `None` when empty.
pub fn mean(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut n = 0usize;
    let total = sum(values.into_iter().inspect(|_| n += 1));
    (n > 0).then(|| total / n as f64)
}

/// Variance with divisor n − ddof (0: population, 1: sample); NaN when n ≤ ddof.
pub fn variance(xs: &[f64], ddof: usize) -> f64 {
    if xs.len() <= ddof {
        return f64::NAN;
    }
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

/// Standard normal distribution function Φ(x).
pub fn norm_cdf(x: f64) -> f64 {
    standard_normal().cdf(x)
}

/// Standard normal density φ(x).
pub fn norm_pdf(x: f64) -> f64 {
    standard_normal().pdf(x)
}

fn standard_normal() -> Normal {
    Normal::new(0.0, 1.0).expect("standard normal parameters are valid")
}

/// The total shape above which `beta_ppf` bisects; statrs's inverse stops returning
/// from about 7.26e6.
const BISECT_SHAPE: f64 = 1e5;

/// Beta(a, b) quantile. Up to a total shape of `BISECT_SHAPE`, statrs's inverse, which
/// matches the reference values within 1e-9; above it, bisection on statrs's distribution
/// function, because the inverse's Newton iteration overflows and never ends for large
/// shapes (D-036).
pub fn beta_ppf(p: f64, a: f64, b: f64) -> f64 {
    assert!(
        (0.0..=1.0).contains(&p) && a > 0.0 && b > 0.0,
        "beta_ppf({p}, {a}, {b})"
    );
    let beta = Beta::new(a, b).expect("shape parameters are positive");
    if a + b <= BISECT_SHAPE {
        return beta.inverse_cdf(p);
    }
    if p == 0.0 || p == 1.0 {
        return p;
    }
    // 200 halvings reach the resolution of a double even for quantiles near 0.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..200 {
        let mid = lo + (hi - lo) / 2.0;
        if mid <= lo || mid >= hi {
            break;
        }
        if beta.cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_beta_quantile_returns_for_large_shapes() {
        // Beta(1, n) has the quantile 1 − (1 − p)^(1/n): the exact upper bound at no events.
        for n in [2e5, 7.26e6, 5e7] {
            let q = beta_ppf(0.975, 1.0, n);
            let exact = 1.0 - 0.025f64.powf(1.0 / n);
            assert!((q - exact).abs() < 1e-12, "{n}: {q} {exact}");
        }
        let beta = Beta::new(500.5, 7e6).unwrap();
        let q = beta_ppf(0.01, 500.5, 7e6);
        assert!((beta.cdf(q) - 0.01).abs() < 1e-9, "{q}");
    }

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
        assert!(variance(&[], 1).is_nan() && variance(&[1.0], 1).is_nan());
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
