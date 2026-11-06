//! The alternative annotator test (D-017) against the reference values, which scipy's
//! one-sided t-test and Benjamini-Yekutieli adjustment produce.

use std::collections::BTreeMap;

use evalsig_core::parity::{self, ParityVerdict, Scale};
use serde_json::Value;

mod common;
use common::{cases, close};

fn values(v: &Value) -> BTreeMap<String, f64> {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, x)| (k.clone(), x.as_f64().unwrap()))
        .collect()
}

#[test]
fn alt_test_cases_match() {
    let cases = cases("alt_test");
    assert_eq!(cases.len(), 5);
    for c in &cases {
        let name = c["name"].as_str().unwrap();
        let i = &c["input"];
        let humans = i["humans"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(r, v)| (r.clone(), values(v)))
            .collect();
        let scale = match i["scale"].as_str().unwrap() {
            "nominal" => Scale::Nominal,
            _ => Scale::Graded,
        };
        let p = parity::test(
            &humans,
            &values(&i["system"]),
            scale,
            i["epsilon"].as_f64().unwrap(),
            parity::Q,
        );
        let e = &c["expected"];
        let verdict = match p.verdict {
            ParityVerdict::HumanParity => "human_parity",
            ParityVerdict::BelowHuman => "below_human",
            ParityVerdict::InsufficientData => "insufficient_data",
        };
        assert_eq!(verdict, e["verdict"], "{name}");
        let want = e["tested"].as_array().unwrap();
        assert_eq!(p.tested.len(), want.len(), "{name}");
        for (got, want) in p.tested.iter().zip(want) {
            assert_eq!(got.rater, want["rater"].as_str().unwrap(), "{name}");
            assert_eq!(got.items as u64, want["items"].as_u64().unwrap(), "{name}");
            close(got.mean_difference, &want["mean_difference"], name);
            close(got.p_value, &want["p_value"], name);
            close(got.adjusted_p_value, &want["adjusted_p_value"], name);
            close(got.advantage, &want["advantage"], name);
            assert_eq!(
                got.system_wins,
                want["system_wins"].as_bool().unwrap(),
                "{name}"
            );
        }
        let skipped: BTreeMap<String, u64> = e["skipped"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_u64().unwrap()))
            .collect();
        let got: BTreeMap<String, u64> = p
            .skipped
            .iter()
            .map(|(k, v)| (k.clone(), *v as u64))
            .collect();
        assert_eq!(got, skipped, "{name}");
        match (p.winning_rate, e["winning_rate"].as_f64()) {
            (Some(g), Some(_)) => close(g, &e["winning_rate"], name),
            (g, w) => assert_eq!(g, w, "{name}"),
        }
        match (p.advantage_probability, e["advantage_probability"].as_f64()) {
            (Some(g), Some(_)) => close(g, &e["advantage_probability"], name),
            (g, w) => assert_eq!(g, w, "{name}"),
        }
    }
}
