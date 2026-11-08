//! Power and sample size (metrics spec §12) and the tables of `docs/sample-size.md`,
//! which `scripts/sample_size.py` also writes; a test checks that both agree.

use crate::numeric::{Running, norm_cdf, norm_ppf, sum};

/// ln(k!) for k = 0 … n.
fn log_factorials(n: usize) -> Vec<f64> {
    let mut lf = Vec::with_capacity(n + 1);
    lf.push(0.0);
    for k in 1..=n {
        lf.push(lf[k - 1] + (k as f64).ln());
    }
    lf
}

fn ln_pmf(lf: &[f64], k: usize, n: usize, p: f64) -> f64 {
    // 0 · ln 0 = 0, so p = 0 or 1 keeps the certain outcome's probability at 1.
    let xlogy = |x: usize, y: f64| if x == 0 { 0.0 } else { x as f64 * y.ln() };
    lf[n] - lf[k] - lf[n - k] + xlogy(k, p) + xlogy(n - k, 1.0 - p)
}

/// Items for an exact one-sided 95% bound below u with no event, ⌈ln α / ln(1 − u)⌉,
/// and the (too optimistic) Wilson count ⌈z²(1 − u)/u⌉.
pub fn zero_event_n(u: f64, alpha: f64) -> (u64, u64) {
    let z = norm_ppf(1.0 - alpha);
    (
        (alpha.ln() / (1.0 - u).ln()).ceil() as u64,
        (z * z * (1.0 - u) / u).ceil() as u64,
    )
}

fn wilson_halfwidth(k: f64, n: f64, z: f64) -> f64 {
    let p = k / n;
    z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / (1.0 + z * z / n)
}

/// The smallest n whose two-sided Wilson interval at rate p has half-width ≤ w; above
/// 200 items the search steps by n/200. `u64::MAX` when no count that fits reaches w.
pub fn n_for_halfwidth(p: f64, w: f64, level: f64) -> u64 {
    let z = norm_ppf(0.5 + level / 2.0);
    let mut n: u64 = 2;
    while wilson_halfwidth((p * n as f64).round_ties_even(), n as f64, z) > w {
        match n.checked_add(if n < 200 { 1 } else { (n / 200).max(1) }) {
            Some(next) => n = next,
            None => return u64::MAX,
        }
    }
    n
}

/// Probabilities below this are skipped when summing the power.
const NEGLIGIBLE: f64 = 1e-15;
/// The search for the exact McNemar n stops here.
const MAX_N: usize = 20_000;

/// Exact McNemar power for one effect, with the per-d rejection probabilities
/// computed once and reused across n.
struct McNemar {
    lf: Vec<f64>,
    psi: f64,
    q: f64,
    alpha: f64,
    two_sided: bool,
    tails: Vec<Option<f64>>,
}

impl McNemar {
    /// p10: share of items only the candidate gets right; p01: only the baseline.
    /// Tables cover n up to `max_n`.
    fn new(p10: f64, p01: f64, alpha: f64, two_sided: bool, max_n: usize) -> Self {
        McNemar {
            lf: log_factorials(max_n),
            psi: p10 + p01,
            q: p10 / (p10 + p01),
            alpha,
            two_sided,
            tails: vec![None; max_n + 1],
        }
    }

    /// P(reject | d discordant pairs). Two-sided: b ≤ k or b ≥ d − k, with k the largest
    /// b whose doubled lower tail under ½ is ≤ α (b ≤ d/2). One-sided: b ≥ c, with c the
    /// smallest b whose upper tail under ½ is ≤ α.
    fn reject(&mut self, d: usize) -> f64 {
        if let Some(t) = self.tails[d] {
            return t;
        }
        let lf = &self.lf;
        let pmf = |b: usize, p: f64| ln_pmf(lf, b, d, p).exp();
        let t = if self.two_sided {
            let (mut k, mut cdf) = (-1_i64, Running::default());
            while k < (d / 2) as i64 {
                let mut next = cdf;
                next.add(pmf((k + 1) as usize, 0.5));
                if (2.0 * next.total()).min(1.0) > self.alpha {
                    break;
                }
                cdf = next;
                k += 1;
            }
            if k < 0 {
                0.0
            } else {
                let k = k as usize;
                sum((0..=k).chain(d - k..=d).map(|b| pmf(b, self.q)))
            }
        } else {
            let mut c = d + 1;
            let mut upper = Running::default();
            while c > 0 {
                let mut next = upper;
                next.add(pmf(c - 1, 0.5));
                if next.total() > self.alpha {
                    break;
                }
                upper = next;
                c -= 1;
            }
            sum((c..=d).map(|b| pmf(b, self.q)))
        };
        self.tails[d] = Some(t);
        t
    }

    fn power(&mut self, n: usize) -> f64 {
        let mut total = Running::default();
        for d in 0..=n {
            let pd = ln_pmf(&self.lf, d, n, self.psi).exp();
            if pd < NEGLIGIBLE {
                continue;
            }
            total.add(pd * self.reject(d));
        }
        total.total()
    }
}

/// Power of the exact McNemar test with n paired items.
pub fn mcnemar_power(n: usize, p10: f64, p01: f64, alpha: f64, two_sided: bool) -> f64 {
    McNemar::new(p10, p01, alpha, two_sided, n).power(n)
}

/// The smallest n from which the exact McNemar power stays at or above `power`: the
/// first n of a run of 20 evaluated points that all reach it (the power is not
/// monotone in n). Steps by 1 below 400 items and by 5 above. `None` past 20 000.
pub fn mcnemar_n_exact(p10: f64, p01: f64, power: f64, alpha: f64, two_sided: bool) -> Option<u64> {
    let mut m = McNemar::new(p10, p01, alpha, two_sided, MAX_N);
    let (mut n, mut streak, mut first) = (10_usize, 0, None);
    while n < MAX_N {
        if m.power(n) >= power {
            first = first.or(Some(n));
            streak += 1;
            if streak >= 20 {
                return first.map(|f| f as u64);
            }
        } else {
            (first, streak) = (None, 0);
        }
        n += if n < 400 { 1 } else { 5 };
    }
    None
}

/// Connor's (1987) approximation, ⌈(z_α √ψ + z_β √(ψ − δ²))² / δ²⌉. `None` without
/// a difference to detect or for shares that are not probabilities.
pub fn mcnemar_n_connor(
    p10: f64,
    p01: f64,
    power: f64,
    alpha: f64,
    two_sided: bool,
) -> Option<u64> {
    let (psi, delta) = (p10 + p01, (p10 - p01).abs());
    if p10 < 0.0 || p01 < 0.0 || psi > 1.0 || delta == 0.0 {
        return None;
    }
    let za = norm_ppf(1.0 - if two_sided { alpha / 2.0 } else { alpha });
    let zb = norm_ppf(power);
    Some(
        ((za * psi.sqrt() + zb * (psi - delta * delta).sqrt()).powi(2) / (delta * delta)).ceil()
            as u64,
    )
}

/// The smallest accuracy difference δ < ψ that n paired items detect with `power`
/// under Connor's approximation, by bisection; `None` when even δ → ψ needs more.
pub fn minimum_detectable_difference(
    n: u64,
    psi: f64,
    power: f64,
    alpha: f64,
    two_sided: bool,
) -> Option<f64> {
    let needs = |delta: f64| {
        mcnemar_n_connor(
            (psi + delta) / 2.0,
            (psi - delta) / 2.0,
            power,
            alpha,
            two_sided,
        )
        .unwrap_or(u64::MAX)
    };
    let (mut lo, mut hi) = (0.0, psi * (1.0 - 1e-9));
    if needs(hi) > n {
        return None;
    }
    for _ in 0..100 {
        let mid = (lo + hi) / 2.0;
        if needs(mid) <= n {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(hi)
}

/// Items after the design effect 1 + (m̄ − 1)ρ of clusters of mean size m̄ and
/// intra-cluster correlation ρ.
pub fn with_design_effect(n: u64, mean_cluster: f64, icc: f64) -> u64 {
    (n as f64 * (1.0 + (mean_cluster - 1.0) * icc)).ceil() as u64
}

/// The per-submission level of a gate with a budget of B submissions, α/B.
pub fn bonferroni(alpha: f64, budget: u32) -> f64 {
    alpha / f64::from(budget.max(1))
}

/// The probability that a difference condition passes on `n` paired items with
/// discordance `psi` when the candidate is truly better by `effect` (positive is better;
/// metrics spec §11.1): the difference's lower bound, with `tail` beyond it, must exceed
/// `threshold` and its point reach `min_effect` (−∞ for a rule without one). The paired
/// difference is taken as normal
/// with standard error √((ψ − effect²)/n), ψ raised to |effect| when smaller, since no
/// more items can change than are discordant. `None` without items or with a tail
/// outside (0, 1).
pub fn difference_pass_probability(
    n: f64,
    psi: f64,
    effect: f64,
    threshold: f64,
    min_effect: f64,
    tail: f64,
) -> Option<f64> {
    if n <= 0.0 || !(tail > 0.0 && tail < 1.0) {
        return None;
    }
    let psi = psi.max(effect.abs());
    let se = ((psi - effect * effect).max(0.0) / n).sqrt();
    let need = (threshold + norm_ppf(1.0 - tail) * se).max(min_effect);
    if se == 0.0 {
        // Every item changes the same way: the difference is the effect itself.
        return Some(if effect > threshold && effect >= min_effect {
            1.0
        } else {
            0.0
        });
    }
    Some(1.0 - norm_cdf((need - effect) / se))
}

/// `docs/sample-size.md`, as `scripts/sample_size.py` writes it.
pub fn table() -> String {
    fn text(l: &mut Vec<String>, lines: &[&str]) {
        l.extend(lines.iter().map(|s| s.to_string()));
    }
    let mut l: Vec<String> = Vec::new();
    text(
        &mut l,
        &[
            "# Sample sizes",
            "",
            "Generated by `scripts/sample_size.py`; `evalsig_core::power` renders the same file.",
            "The unit is the item. Replicates of the same item do not add to the count (see",
            "\"Replicates and item counts\" below).",
            "",
            "## 1. Items to claim a disagreement rate below u with none observed (one-sided 95%)",
            "",
            "| u | Exact (Clopper-Pearson) | Wilson | Rule of thumb (3/u) |",
            "|---|---|---|---|",
        ],
    );
    for u in [0.1, 0.05, 0.02, 0.01, 0.005, 0.001] {
        let (e, w) = zero_event_n(u, 0.05);
        l.push(format!("| {u} | {e} | {w} | {} |", (3.0 / u).ceil()));
    }
    text(
        &mut l,
        &[
            "",
            "- With no events Wilson's upper bound is too small, so it passes with about 10% fewer",
            "  items than the exact method. Gates use the exact method; Wilson is for the two-sided",
            "  interval shown in reports.",
            "",
            "## 2. Items to measure a rate to within ±w (half-width of the 95% interval)",
            "",
            "| Rate p | w=0.10 | w=0.05 | w=0.03 | w=0.02 |",
            "|---|---|---|---|---|",
        ],
    );
    for p in [0.5, 0.2, 0.1, 0.05] {
        let cells: Vec<String> = [0.10, 0.05, 0.03, 0.02]
            .iter()
            .map(|w| n_for_halfwidth(p, *w, 0.95).to_string())
            .collect();
        l.push(format!("| {p} | {} |", cells.join(" | ")));
    }
    text(
        &mut l,
        &[
            "",
            "- Rates whose denominator is one class (hit rate, false-alarm rate) are read with that",
            "  class's count. At 10% prevalence, measuring the positive side to ±0.05 takes about ten",
            "  times the table's value in items overall.",
            "",
            "## 3. Items to detect an accuracy difference δ between two systems on the same items",
            "",
            "Exact McNemar test (two-sided α = 0.05) with power 0.8. ψ is the share of items only one",
            "system gets right (the discordance). Exact sums the power over the binomial",
            "distributions; the approximation is Connor (1987).",
            "",
            "| δ | ψ | Exact | Approximation |",
            "|---|---|---|---|",
        ],
    );
    for delta in [0.10, 0.05, 0.03] {
        for psi in [0.10_f64, 0.20, 0.30] {
            if delta >= psi {
                continue;
            }
            let (p10, p01) = ((psi + delta) / 2.0, (psi - delta) / 2.0);
            let exact = mcnemar_n_exact(p10, p01, 0.8, 0.05, true)
                .map_or_else(|| "None".to_owned(), |n| n.to_string());
            l.push(format!(
                "| {delta} | {psi} | {exact} | {} |",
                mcnemar_n_connor(p10, p01, 0.8, 0.05, true)
                    .map_or_else(|| "None".to_owned(), |n| n.to_string())
            ));
        }
    }
    text(
        &mut l,
        &[
            "",
            "- For the same δ, a larger ψ (the systems fail on different items) needs more items.",
            "  When ψ is unknown, measure it on a small pilot run and pass it to `power`.",
            "",
            "## Replicates and item counts",
            "",
            "When each item's value is the mean of K replicates, with between-item variance σ_b² and",
            "within-item (between-replicate) variance σ_w², the mean of n items has variance",
            "(σ_b² + σ_w²/K) / n. More replicates shrink only the σ_w² term; the σ_b² term shrinks",
            "only with more items. Counting replicates as independent items makes intervals too narrow.",
            "",
        ],
    );
    l.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_levels_follow_the_spec_examples() {
        // metrics spec §12: δ = 0.05, ψ = 0.10, power 0.8, one-sided.
        let (p10, p01) = (0.075, 0.025);
        assert_eq!(mcnemar_n_connor(p10, p01, 0.8, 0.05, false), Some(246));
        assert_eq!(mcnemar_n_connor(0.05, 0.05, 0.8, 0.05, false), None);
        assert_eq!(
            mcnemar_n_connor(p10, p01, 0.8, bonferroni(0.05, 20), false),
            Some(530)
        );
        let d = minimum_detectable_difference(246, 0.10, 0.8, 0.05, false).unwrap();
        assert!(d <= 0.05 && d > 0.049);
        assert_eq!(with_design_effect(100, 3.0, 0.5), 200);
        let p = mcnemar_power(530, p10, p01, bonferroni(0.05, 20), false);
        assert!(p > 0.7 && p < 0.9, "{p}");
    }

    #[test]
    fn a_difference_condition_passes_as_its_effect_allows() {
        // At an effect of exactly min_effect the point condition alone caps it near ½.
        let p = difference_pass_probability(600.0, 0.1, 0.05, 0.0, 0.05, 0.01).unwrap();
        assert!((p - 0.5).abs() < 1e-12, "{p}");
        // A larger true effect clears both parts: 1 − Φ((0.05 − 0.08)/se).
        let se = ((0.1 - 0.08f64.powi(2)) / 600.0).sqrt();
        let p = difference_pass_probability(600.0, 0.1, 0.08, 0.0, 0.05, 0.01).unwrap();
        assert!(
            (p - (1.0 - norm_cdf((0.05 - 0.08) / se))).abs() < 1e-12,
            "{p}"
        );
        // Non-inferiority at no true difference: a margin of 0.03 is about z·se here, so
        // it passes about half the time; a margin of 0.05 almost always.
        let se = (0.1f64 / 600.0).sqrt();
        let p =
            difference_pass_probability(600.0, 0.1, 0.0, -0.03, f64::NEG_INFINITY, 0.01).unwrap();
        let expected = 1.0 - norm_cdf((-0.03 + norm_ppf(0.99) * se) / se);
        assert!(
            (p - expected).abs() < 1e-12 && (p - 0.5).abs() < 0.01,
            "{p}"
        );
        let p =
            difference_pass_probability(600.0, 0.1, 0.0, -0.05, f64::NEG_INFINITY, 0.01).unwrap();
        assert!(p > 0.9, "{p}");
        assert_eq!(
            difference_pass_probability(0.0, 0.1, 0.05, 0.0, 0.0, 0.01),
            None
        );
        assert_eq!(
            difference_pass_probability(100.0, 0.1, 1.0, 0.0, 0.05, 0.01),
            Some(1.0)
        );
    }

    #[test]
    fn every_pair_discordant_is_a_point_mass_not_nan() {
        // ψ = 1, p10 = 0.75: the power is P(Binomial(100, 0.75) rejects), 0.9998529257
        // from scipy, and the exact size is 28.
        let p = mcnemar_power(100, 0.75, 0.25, 0.05, false);
        assert!((p - 0.999_852_925_701_64).abs() < 1e-9, "{p}");
        assert_eq!(mcnemar_n_exact(0.75, 0.25, 0.8, 0.05, false), Some(28));
    }

    #[test]
    fn a_half_width_no_count_can_reach_is_the_largest_count() {
        assert_eq!(n_for_halfwidth(0.5, 0.0, 0.95), u64::MAX);
    }
}
