//! Feature likelihood ratios against the reference values.

use evalsig_core::features::{self, Class, Counts, Thresholds};
use serde_json::Value;
mod common;

use common::{cases, close};

fn counts(v: &Value) -> Counts {
    let f = |i: usize| v[i].as_f64().unwrap();
    Counts {
        a: f(0),
        n_pos: f(1),
        b: f(2),
        n_neg: f(3),
    }
}

#[test]
fn likelihood_ratio_cases_match() {
    let cases = cases("feature_likelihood_ratio");
    assert!(!cases.is_empty());
    for c in &cases {
        let i = &c["input"];
        let n = |k: &str| i[k].as_f64().unwrap();
        let r = features::likelihood_ratio(
            Counts {
                a: n("hit_pos"),
                n_pos: n("n_pos"),
                b: n("hit_neg"),
                n_neg: n("n_neg"),
            },
            0.95,
        )
        .unwrap();
        close(r.lr, &c["expected"]["lr"], "lr");
        close(r.interval.0, &c["expected"]["katz95"][0], "lo");
        close(r.interval.1, &c["expected"]["katz95"][1], "hi");
    }
}

#[test]
fn bh_adjusted_intervals_match() {
    let c = &cases("feature_likelihood_ratio_bh")[0];
    let feats: Vec<Counts> = c["input"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(counts)
        .collect();
    let assessed = features::assess(&feats, 0.95, Thresholds::default());
    let ps: Vec<f64> = assessed.iter().map(|a| a.unwrap().p_value).collect();
    for (p, want) in ps.iter().zip(c["expected"]["p_values"].as_array().unwrap()) {
        close(*p, want, "p");
    }
    close(
        features::adjusted_level(&ps, 0.05),
        &c["expected"]["adjusted_level"],
        "level",
    );
    for (a, want) in assessed
        .iter()
        .zip(c["expected"]["adjusted95"].as_array().unwrap())
    {
        let (lo, hi) = a.unwrap().adjusted_interval;
        close(lo, &want[0], "adjusted lo");
        close(hi, &want[1], "adjusted hi");
    }
    // 12/50 against 3/50 fires 15 times with an interval above 1.
    assert_eq!(assessed[0].unwrap().class, Class::Informative);
    assert_eq!(assessed[3].unwrap().class, Class::Uninformative);
    assert_eq!(assessed[2].unwrap().class, Class::Insufficient);
    let p = features::posterior(4.0, 0.12);
    assert!((p - 4.0 * 0.12 / 0.88 / (1.0 + 4.0 * 0.12 / 0.88)).abs() < 1e-12);
}

#[test]
fn a_feature_firing_on_every_item_has_p_value_one() {
    let all = Counts {
        a: 20.0,
        n_pos: 20.0,
        b: 30.0,
        n_neg: 30.0,
    };
    let assessed = features::assess(&[all], 0.95, Thresholds::default());
    assert_eq!(assessed[0].as_ref().unwrap().p_value, 1.0);
}
