//! The gate's rules against the reference cases: every rule passes and fails.

use std::collections::BTreeSet;

use evalsig_core::gate::{self, Evidence, Rule, Sided};
use serde_json::Value;
mod common;

use common::{cases, close};

fn evidence(e: &Value) -> Evidence {
    let f = |k: &str| e[k].as_f64().unwrap();
    match e["kind"].as_str().unwrap() {
        "value" => Evidence::Value {
            point: f("point"),
            lo: f("lo"),
            hi: f("hi"),
        },
        "delta" => Evidence::Delta {
            point: f("point"),
            lo: f("lo"),
            hi: f("hi"),
        },
        _ => Evidence::Verdict(e["verdict"].as_bool().unwrap()),
    }
}

#[test]
fn each_rule_has_a_passing_and_a_failing_case() {
    let cases = cases("gate_rules");
    let mut seen: BTreeSet<(String, bool)> = BTreeSet::new();
    for c in cases.iter().filter(|c| c["input"]["rule"].is_string()) {
        let i = &c["input"];
        let rule: Rule = serde_json::from_value(i["rule"].clone()).unwrap();
        let out = gate::judge(
            rule,
            i["value"].as_f64().unwrap(),
            i["min_effect"].as_f64(),
            Some(evidence(&i["evidence"])),
        );
        let want = c["expected"]["passed"].as_bool().unwrap();
        assert_eq!(out.passed, Some(want), "{i}");
        seen.insert((i["rule"].as_str().unwrap().to_owned(), want));
    }
    for rule in [
        "lower_bound_at_least",
        "upper_bound_at_most",
        "delta_lower_bound_at_least",
        "delta_upper_bound_at_most",
        "not_worse_than",
        "pass",
    ] {
        assert!(
            seen.contains(&(rule.to_owned(), true)) && seen.contains(&(rule.to_owned(), false)),
            "{rule}"
        );
    }
    // The rule set has exactly these six, all read intervals or verdicts.
    assert!(serde_json::from_value::<Rule>(Value::String("point_at_least".into())).is_err());
}

#[test]
fn the_missing_rate_counts_every_missing_value_with_the_exact_bound() {
    let c = cases("gate_rules")
        .into_iter()
        .find(|c| c["input"]["missing_rate"].is_object())
        .unwrap();
    let m = &c["input"]["missing_rate"];
    let level = m["one_sided_level"].as_f64().unwrap();
    // A one-sided condition at 1 − α/B reads the two-sided interval at 1 − 2α/B.
    let e = gate::missing_rate(
        m["missing"].as_f64().unwrap(),
        m["judgments"].as_f64().unwrap(),
        2.0 * level - 1.0,
        Sided::One,
    )
    .unwrap();
    let Evidence::Value { hi, .. } = e else {
        unreachable!()
    };
    close(hi, &c["expected"]["upper"], "upper");
    let out = gate::judge(Rule::UpperBoundAtMost, 0.05, None, Some(e));
    assert_eq!(out.passed, Some(true));
}
