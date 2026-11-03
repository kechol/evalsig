//! Binary signal-detection metrics against the reference values (absolute 1e-9).

use evalsig_core::sdt::Confusion;
mod common;

use common::{cases, close};

#[test]
fn sdt_binary_cases_match() {
    let cases = cases("sdt_binary");
    assert_eq!(cases.len(), 4);
    for c in &cases {
        let i = &c["input"];
        let (kh, np) = (i["k_hit"].as_f64().unwrap(), i["n_pos"].as_f64().unwrap());
        let (kf, nn) = (i["k_fa"].as_f64().unwrap(), i["n_neg"].as_f64().unwrap());
        let conf = Confusion {
            hits: kh,
            misses: np - kh,
            false_alarms: kf,
            correct_rejections: nn - kf,
        };
        let e = &c["expected"];
        for (name, got) in [
            ("H_raw", conf.hit_rate().unwrap()),
            ("F_raw", conf.false_alarm_rate().unwrap()),
            ("H_ll", conf.corrected_hit_rate()),
            ("F_ll", conf.corrected_false_alarm_rate()),
            ("dprime", conf.dprime()),
            ("c", conf.criterion()),
        ] {
            close(got, &e[name], name);
        }
    }
}
