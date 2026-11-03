//! Power and sample size (metrics spec §12) and the tables of `docs/sample-size.md`,
//! which `scripts/sample_size.py` also writes; a test checks that both agree.

use crate::numeric::norm_ppf;

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
    lf[n] - lf[k] - lf[n - k] + k as f64 * p.ln() + (n - k) as f64 * (1.0 - p).ln()
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
/// 200 items the search steps by n/200.
pub fn n_for_halfwidth(p: f64, w: f64, level: f64) -> u64 {
    let z = norm_ppf(0.5 + level / 2.0);
    let mut n: u64 = 2;
    while wilson_halfwidth((p * n as f64).round_ties_even(), n as f64, z) > w {
        n += if n < 200 { 1 } else { (n / 200).max(1) };
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
    fn new(p10: f64, p01: f64, alpha: f64, two_sided: bool) -> Self {
        McNemar {
            lf: log_factorials(MAX_N),
            psi: p10 + p01,
            q: p10 / (p10 + p01),
            alpha,
            two_sided,
            tails: vec![None; MAX_N + 1],
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
            let (mut k, mut cdf) = (-1_i64, 0.0);
            while k < (d / 2) as i64 {
                let next = cdf + pmf((k + 1) as usize, 0.5);
                if (2.0 * next).min(1.0) > self.alpha {
                    break;
                }
                cdf = next;
                k += 1;
            }
            if k < 0 {
                0.0
            } else {
                let k = k as usize;
                (0..=k).map(|b| pmf(b, self.q)).sum::<f64>()
                    + (d - k..=d).map(|b| pmf(b, self.q)).sum::<f64>()
            }
        } else {
            let mut c = d + 1;
            let mut upper = 0.0;
            while c > 0 {
                let next = upper + pmf(c - 1, 0.5);
                if next > self.alpha {
                    break;
                }
                upper = next;
                c -= 1;
            }
            (c..=d).map(|b| pmf(b, self.q)).sum()
        };
        self.tails[d] = Some(t);
        t
    }

    fn power(&mut self, n: usize) -> f64 {
        let mut total = 0.0;
        for d in 0..=n {
            let pd = ln_pmf(&self.lf, d, n, self.psi).exp();
            if pd < NEGLIGIBLE {
                continue;
            }
            total += pd * self.reject(d);
        }
        total
    }
}

/// Power of the exact McNemar test with n paired items.
pub fn mcnemar_power(n: usize, p10: f64, p01: f64, alpha: f64, two_sided: bool) -> f64 {
    McNemar::new(p10, p01, alpha, two_sided).power(n.min(MAX_N))
}

/// The smallest n from which the exact McNemar power stays at or above `power`: the
/// first n of a run of 20 evaluated points that all reach it (the power is not
/// monotone in n). Steps by 1 below 400 items and by 5 above. `None` past 20 000.
pub fn mcnemar_n_exact(p10: f64, p01: f64, power: f64, alpha: f64, two_sided: bool) -> Option<u64> {
    let mut m = McNemar::new(p10, p01, alpha, two_sided);
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

/// Connor's (1987) approximation, ⌈(z_α √ψ + z_β √(ψ − δ²))² / δ²⌉.
pub fn mcnemar_n_connor(p10: f64, p01: f64, power: f64, alpha: f64, two_sided: bool) -> u64 {
    let (psi, delta) = (p10 + p01, (p10 - p01).abs());
    let za = norm_ppf(1.0 - if two_sided { alpha / 2.0 } else { alpha });
    let zb = norm_ppf(power);
    ((za * psi.sqrt() + zb * (psi - delta * delta).sqrt()).powi(2) / (delta * delta)).ceil() as u64
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

/// The probability that a gate passes with n paired items when the effect is as
/// hypothesised: the one-sided exact McNemar power at α/B.
pub fn gate_pass_probability(n: u64, p10: f64, p01: f64, alpha: f64, budget: u32) -> f64 {
    mcnemar_power(n as usize, p10, p01, bonferroni(alpha, budget), false)
}

/// `docs/sample-size.md`, as `scripts/sample_size.py` writes it.
pub fn table() -> String {
    let mut l: Vec<String> = [
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
    ]
    .map(String::from)
    .to_vec();
    for u in [0.1, 0.05, 0.02, 0.01, 0.005, 0.001] {
        let (e, w) = zero_event_n(u, 0.05);
        l.push(format!("| {u} | {e} | {w} | {} |", (3.0 / u).ceil()));
    }
    l.extend(
        [
            "",
            "- With no events Wilson's upper bound is too small, so it passes with about 10% fewer",
            "  items than the exact method. Gates use the exact method; Wilson is for the two-sided",
            "  interval shown in reports.",
            "",
            "## 2. Items to measure a rate to within ±w (half-width of the 95% interval)",
            "",
            "| Rate p | w=0.10 | w=0.05 | w=0.03 | w=0.02 |",
            "|---|---|---|---|---|",
        ]
        .map(String::from),
    );
    for p in [0.5, 0.2, 0.1, 0.05] {
        let cells: Vec<String> = [0.10, 0.05, 0.03, 0.02]
            .iter()
            .map(|w| n_for_halfwidth(p, *w, 0.95).to_string())
            .collect();
        l.push(format!("| {p} | {} |", cells.join(" | ")));
    }
    l.extend(
        [
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
        ]
        .map(String::from),
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
            ));
        }
    }
    l.extend(
        [
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
        ]
        .map(String::from),
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
        assert_eq!(mcnemar_n_connor(p10, p01, 0.8, 0.05, false), 246);
        assert_eq!(
            mcnemar_n_connor(p10, p01, 0.8, bonferroni(0.05, 20), false),
            530
        );
        let d = minimum_detectable_difference(246, 0.10, 0.8, 0.05, false).unwrap();
        assert!(d <= 0.05 && d > 0.049);
        assert_eq!(with_design_effect(100, 3.0, 0.5), 200);
        let p = gate_pass_probability(530, p10, p01, 0.05, 20);
        assert!(p > 0.7 && p < 0.9, "{p}");
    }
}
