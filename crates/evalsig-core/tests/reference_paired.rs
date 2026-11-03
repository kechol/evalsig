//! Paired and unpaired differences and McNemar's test against the reference values.

use evalsig_core::interval::{Z95, newcombe};
use evalsig_core::paired::{Discordant, mcnemar, tango};
use serde_json::Value;

const REFERENCE: &str = include_str!("../../../tests/fixtures/reference-values.json");

fn close(a: f64, b: &Value, what: &str) {
    let b = b.as_f64().unwrap();
    assert!((a - b).abs() <= 1e-9, "{what}: {a} vs {b}");
}

fn doc() -> Value {
    serde_json::from_str(REFERENCE).unwrap()
}

#[test]
fn tango_intervals_match() {
    let doc = doc();
    let cases = doc["cases"]["paired_difference"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for c in cases {
        let i = &c["input"];
        // In the reference, b is positive only in arm 1 (the candidate B).
        let d = Discordant {
            only_b: i["b"].as_u64().unwrap(),
            only_a: i["c"].as_u64().unwrap(),
            n: i["n"].as_u64().unwrap(),
        };
        close(
            d.difference().unwrap(),
            &c["expected"]["difference"],
            "difference",
        );
        let (lo, hi) = tango(d, Z95).unwrap();
        close(lo, &c["expected"]["tango95"][0], &format!("tango low {i}"));
        close(hi, &c["expected"]["tango95"][1], &format!("tango high {i}"));
    }
}

#[test]
fn newcombe_intervals_match_including_the_published_example() {
    let doc = doc();
    for c in doc["cases"]["unpaired_difference"].as_array().unwrap() {
        let i = &c["input"];
        let f = |k: &str| i[k].as_f64().unwrap();
        let (lo, hi) = newcombe(f("k1"), f("n1"), f("k2"), f("n2"), Z95);
        close(lo, &c["expected"]["newcombe95"][0], "newcombe low");
        close(hi, &c["expected"]["newcombe95"][1], "newcombe high");
        if let Some(lit) = c["expected"]["literature_4dp"].as_array() {
            assert_eq!(
                ((lo * 1e4).round() / 1e4, (hi * 1e4).round() / 1e4),
                (lit[0].as_f64().unwrap(), lit[1].as_f64().unwrap())
            );
        }
    }
}

#[test]
fn mcnemar_matches() {
    let doc = doc();
    for c in doc["cases"]["mcnemar"].as_array().unwrap() {
        let (b, cc) = (
            c["input"]["b"].as_u64().unwrap(),
            c["input"]["c"].as_u64().unwrap(),
        );
        let (exact, mid) = mcnemar(b, cc);
        close(
            exact,
            &c["expected"]["exact_two_sided_p"],
            &format!("exact {b},{cc}"),
        );
        close(
            mid,
            &c["expected"]["mid_p_two_sided"],
            &format!("mid-p {b},{cc}"),
        );
    }
}
