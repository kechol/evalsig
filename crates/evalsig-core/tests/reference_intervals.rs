//! Rate intervals against the reference values (absolute 1e-9).

use evalsig_core::interval::{
    clopper_pearson, clopper_pearson_upper, jeffreys, wilson_two_sided, wilson_upper,
    zero_event_items,
};
use serde_json::Value;

const REFERENCE: &str = include_str!("../../../tests/fixtures/reference-values.json");

fn close(a: f64, b: &Value, what: &str) {
    let b = b.as_f64().unwrap();
    assert!((a - b).abs() <= 1e-9, "{what}: {a} vs {b}");
}

#[test]
fn proportion_intervals_match() {
    let doc: Value = serde_json::from_str(REFERENCE).unwrap();
    let cases = doc["cases"]["proportion_interval"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for c in cases {
        let k = c["input"]["k"].as_f64().unwrap();
        let n = c["input"]["n"].as_f64().unwrap();
        let e = &c["expected"];
        let pairs = [
            ("wilson95", wilson_two_sided(k, n, 0.95)),
            ("clopper_pearson95", clopper_pearson(k, n, 0.95)),
            ("jeffreys95", jeffreys(k, n, 0.95)),
        ];
        for (name, (lo, hi)) in pairs {
            close(lo, &e[name][0], &format!("{name} low {k}/{n}"));
            close(hi, &e[name][1], &format!("{name} high {k}/{n}"));
        }
        close(
            wilson_upper(k, n, 0.95),
            &e["wilson_one_sided95_upper"],
            "wilson one-sided",
        );
        close(
            clopper_pearson_upper(k, n, 0.95),
            &e["clopper_pearson_one_sided95_upper"],
            "exact one-sided",
        );
    }
}

#[test]
fn zero_event_sample_sizes_reproduce() {
    let doc: Value = serde_json::from_str(REFERENCE).unwrap();
    let mut seen = 0;
    for c in doc["cases"]["misc"].as_array().unwrap() {
        let Some(u) = c["input"]["upper_bound"].as_f64() else {
            continue;
        };
        let level = c["input"]["confidence_one_sided"].as_f64().unwrap();
        let (exact, wilson) = zero_event_items(u, level);
        assert_eq!(
            exact,
            c["expected"]["n_exact"].as_u64().unwrap(),
            "exact {u}"
        );
        assert_eq!(
            wilson,
            c["expected"]["n_wilson"].as_u64().unwrap(),
            "wilson {u}"
        );
        seen += 1;
    }
    assert_eq!(seen, 2);
    // The specification's headline numbers: below 1% needs 299 (exact) and 268 (Wilson).
    assert_eq!(zero_event_items(0.01, 0.95), (299, 268));
}
