//! The normal quantile against the reference values (absolute 1e-9); the Beta
//! quantile is covered by the exact and Jeffreys intervals in `reference_intervals`.

use evalsig_core::numeric::norm_ppf;
use serde_json::Value;

const REFERENCE: &str = include_str!("../../../tests/fixtures/reference-values.json");

fn close(a: f64, b: &Value, what: &str) {
    let b = b.as_f64().unwrap();
    assert!((a - b).abs() <= 1e-9, "{what}: {a} vs {b}");
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
