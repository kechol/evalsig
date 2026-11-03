//! Intervals for rates (metrics spec §1).

/// z for a two-sided 95% interval, Φ⁻¹(0.975).
pub const Z95: f64 = 1.959_963_984_540_054;

/// Wilson score interval for k successes out of n, at the given z.
pub fn wilson(k: u64, n: u64, z: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (k, n) = (k as f64, n as f64);
    let p = k / n;
    let z2 = z * z;
    let d = 1.0 + z2 / n;
    let c = (p + z2 / (2.0 * n)) / d;
    let h = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

/// Newcombe's hybrid score interval (method 10) for p_a − p_b of two independent
/// proportions, from Wilson intervals at the given z.
pub fn newcombe(ka: u64, na: u64, kb: u64, nb: u64, z: f64) -> (f64, f64) {
    let (pa, pb) = (ka as f64 / na.max(1) as f64, kb as f64 / nb.max(1) as f64);
    let (la, ua) = wilson(ka, na, z);
    let (lb, ub) = wilson(kb, nb, z);
    let d = pa - pb;
    (
        d - ((pa - la).powi(2) + (ub - pb).powi(2)).sqrt(),
        d + ((ua - pa).powi(2) + (pb - lb).powi(2)).sqrt(),
    )
}
