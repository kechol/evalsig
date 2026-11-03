//! The distribution functions against the reference values (absolute 1e-9).

use evalsig_core::numeric::{beta_ppf, norm_ppf};
use serde_json::Value;

const REFERENCE: &str = include_str!("../../../tests/fixtures/reference-values.json");

fn close(a: f64, b: &Value, what: &str) {
    let b = b.as_f64().unwrap();
    assert!((a - b).abs() <= 1e-9, "{what}: {a} vs {b}");
}

#[test]
fn beta_quantiles_reproduce_exact_and_jeffreys_intervals() {
    let doc: Value = serde_json::from_str(REFERENCE).unwrap();
    for case in doc["cases"]["proportion_interval"].as_array().unwrap() {
        let k = case["input"]["k"].as_f64().unwrap();
        let n = case["input"]["n"].as_f64().unwrap();
        let e = &case["expected"];
        let cp_lo = if k == 0.0 {
            0.0
        } else {
            beta_ppf(0.025, k, n - k + 1.0)
        };
        let cp_hi = if k == n {
            1.0
        } else {
            beta_ppf(0.975, k + 1.0, n - k)
        };
        close(
            cp_lo,
            &e["clopper_pearson95"][0],
            &format!("CP low {k}/{n}"),
        );
        close(
            cp_hi,
            &e["clopper_pearson95"][1],
            &format!("CP high {k}/{n}"),
        );
        let j_lo = if k == 0.0 {
            0.0
        } else {
            beta_ppf(0.025, k + 0.5, n - k + 0.5)
        };
        let j_hi = if k == n {
            1.0
        } else {
            beta_ppf(0.975, k + 0.5, n - k + 0.5)
        };
        close(j_lo, &e["jeffreys95"][0], &format!("Jeffreys low {k}/{n}"));
        close(j_hi, &e["jeffreys95"][1], &format!("Jeffreys high {k}/{n}"));
        let one = if k == n {
            1.0
        } else {
            beta_ppf(0.95, k + 1.0, n - k)
        };
        close(
            one,
            &e["clopper_pearson_one_sided95_upper"],
            &format!("CP one-sided {k}/{n}"),
        );
    }
}

#[test]
fn normal_quantiles_reproduce_dprime() {
    let doc: Value = serde_json::from_str(REFERENCE).unwrap();
    for case in doc["cases"]["sdt_binary"].as_array().unwrap() {
        let e = &case["expected"];
        let (h, f) = (e["H_ll"].as_f64().unwrap(), e["F_ll"].as_f64().unwrap());
        close(norm_ppf(h) - norm_ppf(f), &e["dprime"], "d'");
    }
}
