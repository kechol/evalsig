//! Agreement metrics against the reference values.

use evalsig_core::agreement::{self, Level};
use serde_json::Value;
mod common;

use common::{cases, close};

fn table(v: &Value) -> Vec<Vec<f64>> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        })
        .collect()
}

#[test]
fn kappa_and_krippendorff_alpha_match() {
    let cases = cases("agreement");
    assert_eq!(cases.len(), 2);
    let kappa = agreement::cohen_kappa(&table(&cases[0]["input"]["confusion"])).unwrap();
    close(kappa, &cases[0]["expected"]["cohen_kappa"], "kappa");

    let coders = cases[1]["input"]["coders"].as_object().unwrap();
    let width = coders.values().next().unwrap().as_array().unwrap().len();
    let units: Vec<Vec<f64>> = (0..width)
        .map(|u| coders.values().filter_map(|c| c[u].as_f64()).collect())
        .collect();
    let e = &cases[1]["expected"];
    for (level, key) in [
        (Level::Nominal, "krippendorff_alpha_nominal"),
        (Level::Ordinal, "krippendorff_alpha_ordinal"),
        (Level::Interval, "krippendorff_alpha_interval"),
    ] {
        close(
            agreement::krippendorff_alpha(&units, level).unwrap(),
            &e[key],
            key,
        );
    }
}

#[test]
fn prevalence_robust_measures_match() {
    let cases = cases("agreement_prevalence_robust");
    assert_eq!(cases.len(), 3);
    for c in &cases {
        let t = table(&c["input"]["table"]);
        let e = &c["expected"];
        close(
            agreement::cohen_kappa(&t).unwrap(),
            &e["cohen_kappa"],
            "kappa",
        );
        close(agreement::gwet_ac1(&t).unwrap(), &e["gwet_ac1"], "ac1");
        let (pos, neg) = agreement::specific_agreement(&t).unwrap();
        close(pos.unwrap(), &e["positive_agreement"], "positive");
        close(neg.unwrap(), &e["negative_agreement"], "negative");
    }
}

#[test]
fn ordinal_measures_match() {
    let c = &cases("agreement_ordinal")[0];
    let t = table(&c["input"]["table"]);
    close(
        agreement::quadratic_weighted_kappa(&t).unwrap(),
        &c["expected"]["quadratic_weighted_kappa"],
        "weighted kappa",
    );
    let xs = |k: &str| -> Vec<f64> {
        c["input"][k]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect()
    };
    close(
        agreement::spearman(&xs("x"), &xs("y")).unwrap(),
        &c["expected"]["spearman_rho"],
        "spearman",
    );
}
