//! ROC, AUC, DeLong, z-ROC and calibration against the reference values.

use evalsig_core::roc::{self, Weighted};
use serde_json::Value;
mod common;

use common::{cases, close};

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn unit(xs: &[f64]) -> Vec<Weighted> {
    xs.iter()
        .map(|&score| Weighted { score, weight: 1.0 })
        .collect()
}

/// Ordinal histograms (strongest level first) as scores, the strongest the largest.
fn scores(hist: &[u64]) -> Vec<f64> {
    let k = hist.len();
    hist.iter()
        .enumerate()
        .flat_map(|(i, n)| std::iter::repeat_n((k - i) as f64, *n as usize))
        .collect()
}

#[test]
fn ordinal_roc_cases_match() {
    for c in cases("ordinal_roc") {
        let hist = |k: &str| -> Vec<u64> {
            c["input"][k]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap())
                .collect()
        };
        let (hp, hn) = (hist("hist_pos"), hist("hist_neg"));
        let (pos, neg) = (unit(&scores(&hp)), unit(&scores(&hn)));
        close(roc::auc(&pos, &neg).unwrap(), &c["expected"]["auc"], "auc");
        let points = roc::roc_points(&pos, &neg);
        let expected = c["expected"]["roc_points_FH"].as_array().unwrap();
        // The reference lists the interior thresholds only.
        for (got, want) in points[1..points.len() - 1].iter().zip(expected) {
            close(got.0, &want[0], "F");
            close(got.1, &want[1], "H");
        }
        assert_eq!(roc::zroc_points(&hp, &hn).len(), hp.len() - 1);
    }
}

#[test]
fn delong_cases_match() {
    let cases = cases("delong");
    assert_eq!(cases.len(), 3);
    for c in &cases[..2] {
        let d = roc::delong(
            &floats(&c["input"]["pos"]),
            &floats(&c["input"]["neg"]),
            0.95,
        )
        .unwrap();
        close(d.auc, &c["expected"]["auc"], "auc");
        close(d.variance, &c["expected"]["variance"], "variance");
        let (lo, hi) = d.interval.unwrap();
        close(lo, &c["expected"]["logit95"][0], "lo");
        close(hi, &c["expected"]["logit95"][1], "hi");
    }
    let c = &cases[2];
    let labels: Vec<bool> = c["input"]["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l == 1)
        .collect();
    let p = roc::delong_paired(
        &labels,
        &floats(&c["input"]["a"]),
        &floats(&c["input"]["b"]),
        0.95,
    )
    .unwrap();
    let e = &c["expected"];
    close(p.auc_a, &e["auc_a"], "auc_a");
    close(p.auc_b, &e["auc_b"], "auc_b");
    close(p.difference, &e["difference"], "difference");
    close(p.variance, &e["variance_difference"], "variance");
    close(p.z, &e["z"], "z");
    close(p.p_two_sided, &e["p_two_sided"], "p");
    close(p.interval.0, &e["difference95"][0], "lo");
    close(p.interval.1, &e["difference95"][1], "hi");
}

#[test]
fn calibration_cases_match() {
    for c in cases("calibration") {
        let scores = floats(&c["input"]["scores"]);
        let (brier, bins) = roc::calibration(
            &scores,
            &floats(&c["input"]["labels"]),
            &vec![1.0; scores.len()],
        )
        .unwrap();
        close(brier, &c["expected"]["brier"], "brier");
        let want = c["expected"]["bins"].as_array().unwrap();
        assert_eq!(bins.len(), want.len());
        for (b, w) in bins.iter().zip(want) {
            assert_eq!(b.bin as u64, w["bin"].as_u64().unwrap());
            assert_eq!(b.n as u64, w["n"].as_u64().unwrap());
            close(b.mean_score, &w["mean_score"], "mean");
            close(b.positive_share, &w["positive_share"], "share");
        }
    }
}

#[test]
fn clustered_auc_variance_matches() {
    let c = &cases("auc_clustered")[0];
    let pairs = |k: &str| -> Vec<(f64, usize)> {
        c["input"][k]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p[0].as_f64().unwrap(), p[1].as_u64().unwrap() as usize))
            .collect()
    };
    let d = evalsig_core::roc::delong_clustered(&pairs("pos"), &pairs("neg"), 0.95).unwrap();
    close(d.auc, &c["expected"]["auc"], "auc");
    close(d.variance, &c["expected"]["variance"], "variance");
}
