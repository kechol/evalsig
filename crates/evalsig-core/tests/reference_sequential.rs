//! Confidence sequences against the reference values.

use evalsig_core::sequential;
mod common;

use common::{cases, close};

#[test]
fn confidence_sequence_cases_match() {
    let cases = cases("confidence_sequence");
    assert_eq!(cases.len(), 2);
    for c in &cases {
        let xs: Vec<f64> = c["input"]["x"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let cs = sequential::bounded_mean(&xs, c["input"]["level"].as_f64().unwrap());
        for t in [1usize, 10, 50, 200] {
            let e = &c["expected"][format!("t{t}")];
            close(cs[t - 1].0, &e[0], "lower");
            close(cs[t - 1].1, &e[1], "upper");
        }
        // Each interval lies inside the previous one.
        assert!(cs.windows(2).all(|w| w[1].0 >= w[0].0 && w[1].1 <= w[0].1));
    }
}

#[test]
fn differences_map_through_the_unit_interval() {
    let ds = vec![0.2; 400];
    let cs = sequential::difference(&ds, 0.95);
    let (lo, hi) = *cs.last().unwrap();
    assert!(lo <= 0.2 && 0.2 <= hi && lo > -1.0 && hi < 1.0);
}
