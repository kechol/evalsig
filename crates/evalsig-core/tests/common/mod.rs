//! Shared access to the reference values and their tolerance.

#![allow(dead_code)]

use serde_json::Value;

const REFERENCE: &str = include_str!("../../../../tests/fixtures/reference-values.json");

/// Absolute tolerance the reference values require.
pub const TOLERANCE: f64 = 1e-9;

pub fn reference() -> Value {
    serde_json::from_str(REFERENCE).expect("reference values are JSON")
}

pub fn cases(family: &str) -> Vec<Value> {
    reference()["cases"][family]
        .as_array()
        .unwrap_or_else(|| panic!("no family {family}"))
        .clone()
}

pub fn close(got: f64, want: &Value, what: &str) {
    let want = want
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: expected a number, got {want}"));
    assert!((got - want).abs() <= TOLERANCE, "{what}: {got} vs {want}");
}
