//! PPI++ for a mean against the reference values.

use evalsig_core::ppi;
mod common;

use common::{cases, close};

#[test]
fn ppi_mean_cases_match() {
    let cases = cases("ppi_mean");
    assert_eq!(cases.len(), 3);
    for c in &cases {
        let i = &c["input"];
        let xs = |k: &str| -> Vec<f64> {
            i[k].as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        };
        let labelled: Vec<(f64, f64)> = xs("y").into_iter().zip(xs("f")).collect();
        let p = ppi::mean_estimate(
            &labelled,
            &xs("f_unlabelled"),
            i["level"].as_f64().unwrap(),
            i["lambda"].as_f64(),
        )
        .unwrap();
        let e = &c["expected"];
        close(p.estimate, &e["estimate"], "estimate");
        close(p.se, &e["se"], "se");
        close(p.lambda, &e["lambda"], "lambda");
        close(p.interval.0, &e["ci95"][0], "lo");
        close(p.interval.1, &e["ci95"][1], "hi");
    }
}
