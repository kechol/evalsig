//! Coverage of the intervals (SPEC §13.2): Wilson, exact (Clopper-Pearson), Tango and
//! the cluster bootstrap must contain the true value 93–97% of the time, at the
//! display level (two-sided 95%) and at a gate's one-sided level 1 − α/B with B = 1.
//! At B = 5 (one-sided 99%) the miscoverage must stay within 2·α/B.
//!
//! Binomial coverage is summed exactly over k and averaged over 21 true rates within
//! ±0.02 of each grid point, which smooths the oscillation of discrete coverage
//! (Brown, Cai & DasGupta 2001). Tango's coverage is summed exactly over the
//! multinomial of discordant pairs; the bootstrap's is a seeded Monte Carlo estimate.
//!
//! Slow in debug builds; run with
//! `cargo test --release -p evalsig-core --test coverage -- --ignored --nocapture`.

use evalsig_core::bootstrap;
use evalsig_core::interval::{Z95, clopper_pearson, clopper_pearson_upper, wilson, wilson_upper};
use evalsig_core::numeric::{Rng, mean, norm_ppf};
use evalsig_core::paired::{Discordant, tango};

const ALPHA: f64 = 0.05;
const TARGET: (f64, f64) = (0.93, 0.97);

fn ln_choose(n: u64, k: u64) -> f64 {
    let lf = |m: u64| (1..=m).map(|i| (i as f64).ln()).sum::<f64>();
    lf(n) - lf(k) - lf(n - k)
}

/// Exact coverage of a rate interval at true rate p with n items.
fn binomial_coverage(n: u64, p: f64, covers: impl Fn(f64, f64, f64) -> bool) -> f64 {
    (0..=n)
        .map(|k| {
            let pk = (ln_choose(n, k) + k as f64 * p.ln() + (n - k) as f64 * (1.0 - p).ln()).exp();
            if covers(k as f64, n as f64, p) {
                pk
            } else {
                0.0
            }
        })
        .sum()
}

fn smoothed(n: u64, p0: f64, covers: &dyn Fn(f64, f64, f64) -> bool) -> f64 {
    mean((0..21).map(|i| binomial_coverage(n, p0 - 0.02 + 0.002 * f64::from(i), covers))).unwrap()
}

/// A named check of whether the interval for (k, n) covers p.
type Method = (&'static str, Box<dyn Fn(f64, f64, f64) -> bool>);

struct Row {
    method: &'static str,
    level: &'static str,
    setting: String,
    coverage: f64,
    bounds: (f64, f64),
}

fn check(rows: &[Row]) {
    eprintln!("| method | level | setting | coverage |\n|---|---|---|---|");
    for r in rows {
        eprintln!(
            "| {} | {} | {} | {:.4} |",
            r.method, r.level, r.setting, r.coverage
        );
    }
    let bad: Vec<&Row> = rows
        .iter()
        .filter(|r| r.coverage < r.bounds.0 || r.coverage > r.bounds.1)
        .collect();
    assert!(
        bad.is_empty(),
        "outside the target: {:?}",
        bad.iter()
            .map(|r| (r.method, r.level, &r.setting, r.coverage))
            .collect::<Vec<_>>()
    );
}

/// The one-sided level and the accepted coverage for a gate with budget B.
fn gate(b: u32) -> (f64, (f64, f64)) {
    let level = 1.0 - ALPHA / f64::from(b);
    if b == 1 {
        (level, TARGET)
    } else {
        (level, (1.0 - 2.0 * ALPHA / f64::from(b), 1.0))
    }
}

#[test]
#[ignore = "coverage simulation; run in the scheduled job"]
fn binomial_intervals_cover() {
    let mut rows = Vec::new();
    for n in [100u64, 300] {
        for p0 in [0.1, 0.3, 0.5] {
            let setting = format!("n={n}, p={p0}");
            let two_sided: [Method; 2] = [
                (
                    "wilson",
                    Box::new(|k, n, p| {
                        let (lo, hi) = wilson(k, n, Z95);
                        lo <= p && p <= hi
                    }),
                ),
                (
                    "exact",
                    Box::new(|k, n, p| {
                        let (lo, hi) = clopper_pearson(k, n, 0.95);
                        lo <= p && p <= hi
                    }),
                ),
            ];
            for (method, f) in &two_sided {
                rows.push(Row {
                    method,
                    level: "two-sided 95%",
                    setting: setting.clone(),
                    coverage: smoothed(n, p0, f.as_ref()),
                    bounds: TARGET,
                });
            }
            for b in [1u32, 5] {
                let (level, bounds) = gate(b);
                let upper: [Method; 2] = [
                    (
                        "wilson",
                        Box::new(move |k, n, p| p <= wilson_upper(k, n, level)),
                    ),
                    (
                        "exact",
                        Box::new(move |k, n, p| p <= clopper_pearson_upper(k, n, level)),
                    ),
                ];
                for (method, f) in &upper {
                    rows.push(Row {
                        method,
                        level: if b == 1 {
                            "one-sided α/B, B=1"
                        } else {
                            "one-sided α/B, B=5"
                        },
                        setting: setting.clone(),
                        coverage: smoothed(n, p0, f.as_ref()),
                        bounds,
                    });
                }
            }
        }
    }
    check(&rows);
}

/// Exact coverage of Tango's interval for p_B − p_A with n paired items, where a share
/// pa of items is right only in A and pb only in B.
fn tango_coverage(n: u64, pa: f64, pb: f64, z: f64, upper_only: bool) -> f64 {
    let truth = pb - pa;
    let mut total = 0.0;
    for a in 0..=n {
        for b in 0..=(n - a) {
            let rest = n - a - b;
            let lp = ln_choose(n, a)
                + ln_choose(n - a, b)
                + a as f64 * pa.ln()
                + b as f64 * pb.ln()
                + rest as f64 * (1.0 - pa - pb).ln();
            let p = lp.exp();
            if p < 1e-14 {
                continue;
            }
            let Some((lo, hi)) = tango(
                Discordant {
                    only_a: a,
                    only_b: b,
                    n,
                },
                z,
            ) else {
                continue;
            };
            if (upper_only || lo <= truth) && truth <= hi {
                total += p;
            }
        }
    }
    total
}

#[test]
#[ignore = "coverage simulation; run in the scheduled job"]
fn tango_covers() {
    let mut rows = Vec::new();
    for n in [100u64, 300] {
        for (pa, pb) in [(0.05, 0.10), (0.10, 0.10), (0.10, 0.20)] {
            let setting = format!("n={n}, only A {pa}, only B {pb}");
            rows.push(Row {
                method: "tango",
                level: "two-sided 95%",
                setting: setting.clone(),
                coverage: tango_coverage(n, pa, pb, Z95, false),
                bounds: TARGET,
            });
            for b in [1u32, 5] {
                let (level, bounds) = gate(b);
                // The upper end of the interval at z = Φ⁻¹(level) is the one-sided bound.
                rows.push(Row {
                    method: "tango",
                    level: if b == 1 {
                        "one-sided α/B, B=1"
                    } else {
                        "one-sided α/B, B=5"
                    },
                    setting: setting.clone(),
                    coverage: tango_coverage(n, pa, pb, norm_ppf(level), true),
                    bounds,
                });
            }
        }
    }
    check(&rows);
}

#[test]
#[ignore = "coverage simulation; run in the scheduled job"]
fn cluster_bootstrap_covers() {
    const DATASETS: usize = 1000;
    const RESAMPLES: u32 = 1000;
    let mut rows = Vec::new();
    for (clusters, size) in [(60usize, 5usize), (150, 3)] {
        // Cluster rates spread around 0.3 (intra-cluster correlation about 0.1).
        let truth = 0.3;
        let mut rng = Rng::new(20261103 + clusters as u64);
        let mut hits = [0usize; 3];
        for _ in 0..DATASETS {
            let totals: Vec<(f64, f64)> = (0..clusters)
                .map(|_| {
                    // Uniform with standard deviation 0.15.
                    let p = truth + 0.15 * 3f64.sqrt() * (rng.next_f64() * 2.0 - 1.0);
                    let k = (0..size).filter(|_| rng.next_f64() < p).count();
                    (k as f64, size as f64)
                })
                .collect();
            let ratio = |drawn: &[usize]| {
                let (k, n) = drawn
                    .iter()
                    .fold((0.0, 0.0), |(k, n), &i| (k + totals[i].0, n + totals[i].1));
                (n > 0.0).then(|| k / n)
            };
            let (reps, _) = bootstrap::replicates(clusters, None, RESAMPLES, &mut rng, ratio);
            let mut sorted = reps.clone();
            sorted.sort_by(f64::total_cmp);
            let (lo, hi) = bootstrap::percentile_interval(reps, 0.95).unwrap();
            hits[0] += usize::from(lo <= truth && truth <= hi);
            for (slot, b) in [(1, 1u32), (2, 5)] {
                let upper = bootstrap::quantile(&sorted, gate(b).0);
                hits[slot] += usize::from(truth <= upper);
            }
        }
        let setting = format!("{clusters} clusters of {size}, {DATASETS} datasets");
        for (slot, level, bounds) in [
            (0, "two-sided 95%", TARGET),
            (1, "one-sided α/B, B=1", gate(1).1),
            (2, "one-sided α/B, B=5", gate(5).1),
        ] {
            rows.push(Row {
                method: "cluster bootstrap",
                level,
                setting: setting.clone(),
                coverage: hits[slot] as f64 / DATASETS as f64,
                bounds,
            });
        }
    }
    check(&rows);
}
