//! Expected cost and thresholds against the reference values.

use evalsig_core::cost;
use serde_json::Value;
mod common;

use common::{cases, close};

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn matrix2(v: &Value) -> [[f64; 2]; 2] {
    let m: Vec<Vec<f64>> = v.as_array().unwrap().iter().map(floats).collect();
    [[m[0][0], m[0][1]], [m[1][0], m[1][1]]]
}

#[test]
fn cost_cases_match() {
    let cases = cases("cost");
    let c = &cases[0];
    let (pos, neg) = (
        floats(&c["input"]["pos_by_level"]),
        floats(&c["input"]["neg_by_level"]),
    );
    let pi = c["input"]["prevalence"].as_f64().unwrap();
    let matrix: Vec<Vec<f64>> = c["input"]["matrix"]
        .as_array()
        .unwrap()
        .iter()
        .map(floats)
        .collect();
    close(
        cost::expected_cost(&pos, &neg, &matrix, pi).unwrap(),
        &c["expected"]["expected_cost"],
        "ec",
    );
    let binary = matrix2(&c["input"]["binary_matrix"]);
    let ecs = cost::threshold_costs(&pos, &neg, &binary, pi).unwrap();
    for ((_, got), want) in ecs
        .iter()
        .zip(c["expected"]["threshold_costs"].as_array().unwrap())
    {
        close(*got, want, "threshold ec");
    }
    let (best, _) = cost::optimal_threshold(&pos, &neg, &binary, pi).unwrap();
    assert_eq!(
        best as u64,
        c["expected"]["optimal_threshold"].as_u64().unwrap()
    );
    let s = cost::threshold_sensitivity(&pos, &neg, pi, [1.0, 20.0]).unwrap();
    assert_eq!(
        s.at_low as u64,
        c["expected"]["optimal_at_low"].as_u64().unwrap()
    );
    assert_eq!(
        s.at_high as u64,
        c["expected"]["optimal_at_high"].as_u64().unwrap()
    );
    assert!(s.changes);

    let c = &cases[1];
    let pair = |k: &str| {
        let v = floats(&c["input"][k]);
        (v[0], v[1])
    };
    let d = cost::difference_sensitivity(
        pair("a"),
        pair("b"),
        c["input"]["prevalence"].as_f64().unwrap(),
        [1.0, 20.0],
    );
    close(d.at_low, &c["expected"]["difference_at_low"], "low");
    close(d.at_high, &c["expected"]["difference_at_high"], "high");
    assert_eq!(
        d.sign_changes,
        c["expected"]["sign_changes"].as_bool().unwrap()
    );
}

#[test]
fn posterior_and_lr_thresholds_match() {
    for c in cases("misc")
        .iter()
        .filter(|c| c["input"].get("cost_fp").is_some())
    {
        let (fp, fn_, pi) = (
            c["input"]["cost_fp"].as_f64().unwrap(),
            c["input"]["cost_fn"].as_f64().unwrap(),
            c["input"]["prevalence"].as_f64().unwrap(),
        );
        close(
            cost::posterior_threshold(fp, fn_),
            &c["expected"]["posterior_threshold"],
            "p*",
        );
        close(
            cost::lr_threshold(fp, fn_, pi),
            &c["expected"]["lr_threshold"],
            "LR*",
        );
    }
}
