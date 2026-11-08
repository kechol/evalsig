//! Binary signal-detection metrics against the reference values (absolute 1e-9): the
//! raw rates, d′ and c through the report pipeline, the corrected rates from the counts.

use evalsig_core::report::{Report, Sections, Settings};
use evalsig_core::sdt::Confusion;
mod common;

use common::{cases, close};

/// A binary run with `k_hit` of `n_pos` positives and `k_fa` of `n_neg` negatives
/// judged positive.
fn report_of(k_hit: usize, n_pos: usize, k_fa: usize, n_neg: usize) -> Report {
    let mut lines = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for i in 0..n_pos + n_neg {
        let positive = i < n_pos;
        let judged = if positive {
            i < k_hit
        } else {
            i - n_pos < k_fa
        };
        lines.push(format!(r#"{{"record":"item","item_id":"t{i}"}}"#));
        lines.push(format!(
            r#"{{"record":"label","item_id":"t{i}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
        ));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"t{i}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{judged}}}"#
        ));
    }
    common::report_of(
        &lines,
        &Settings {
            sections: Sections {
                detail: true,
                ..Sections::default()
            },
            ..common::settings()
        },
    )
}

#[test]
fn sdt_binary_cases_match() {
    let cases = cases("sdt_binary");
    assert_eq!(cases.len(), 4);
    for c in &cases {
        let i = &c["input"];
        let n = |k: &str| i[k].as_u64().unwrap() as usize;
        let (kh, np, kf, nn) = (n("k_hit"), n("n_pos"), n("k_fa"), n("n_neg"));
        let r = report_of(kh, np, kf, nn);
        let metric = |name: &str| {
            r.metrics
                .iter()
                .find(|m| m.metric == name)
                .and_then(|m| m.value)
                .unwrap()
        };
        let sdt = r.sdt.as_ref().unwrap();
        let conf = Confusion {
            hits: kh as f64,
            misses: (np - kh) as f64,
            false_alarms: kf as f64,
            correct_rejections: (nn - kf) as f64,
        };
        let e = &c["expected"];
        for (name, got) in [
            ("H_raw", metric("hit_rate")),
            ("F_raw", metric("false_alarm_rate")),
            ("H_ll", conf.corrected_hit_rate()),
            ("F_ll", conf.corrected_false_alarm_rate()),
            ("dprime", sdt.dprime),
            ("c", sdt.criterion),
        ] {
            close(got, &e[name], name);
        }
    }
}
