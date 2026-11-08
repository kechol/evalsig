//! Paired and unpaired differences and Tango's score test against the reference values.

use evalsig_core::interval::{Z95, newcombe};
use evalsig_core::paired::{Discordant, tango, tango_p};
mod common;

use common::{cases, close};

#[test]
fn tango_intervals_match() {
    let cases = cases("paired_difference");
    assert_eq!(cases.len(), 5);
    for c in &cases {
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
        close(
            tango_p(d),
            &c["expected"]["score_p"],
            &format!("score p {i}"),
        );
    }
}

#[test]
fn newcombe_intervals_match_including_the_published_example() {
    for c in cases("unpaired_difference") {
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
