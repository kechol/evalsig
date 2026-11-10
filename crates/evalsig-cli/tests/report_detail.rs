//! The detail tier of `evalsig report` (SPEC §6, §10.1): `--features`, `--reference`
//! modes, `--reference human --proxy`, and the ROC and calibration outputs.

mod common;

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .arg("--project")
        .arg(dir);
    c
}

/// A score run on 60 items in 20 clusters: positives score high, a rule fires mostly
/// on positives, two humans label every other item (inclusion probability ½ when
/// `random`; the second disagrees on every tenth item when `split`), and a model rater
/// labels every item, wrongly on every seventh.
fn records(random: bool, split: bool) -> String {
    let mut lines = vec![
        r#"{"record":"run","run_id":"s","system":{"id":"sys","version":"1"},"output_type":"score"}"#
            .to_owned(),
    ];
    for i in 0..60 {
        let positive = i % 3 == 0;
        let score = if positive {
            0.55 + (i % 7) as f64 * 0.06
        } else {
            0.05 + (i % 11) as f64 * 0.05
        };
        let fired = if positive { i % 4 != 0 } else { i % 5 == 0 };
        let prob = if random {
            r#","label_inclusion_prob":0.5"#
        } else {
            ""
        };
        lines.push(format!(
            r#"{{"record":"item","item_id":"i{i:02}","cluster":"c{}"{prob}}}"#,
            i / 3
        ));
        lines.push(format!(
            r#"{{"record":"judgment","item_id":"i{i:02}","run_id":"s","gen_replicate":0,"output_type":"score","value":{score:.2},"features":[{{"feature_id":"rule.a","fired":{fired}}}]}}"#
        ));
        let model = if i % 7 == 0 { !positive } else { positive };
        lines.push(format!(
            r#"{{"record":"label","item_id":"i{i:02}","rater_id":"model:judge","source":"model","value":{model},"version":1}}"#
        ));
        if i % 2 == 0 {
            for (rater, flip) in [("user:ana", false), ("user:ben", split && i % 10 == 0)] {
                lines.push(format!(
                    r#"{{"record":"label","item_id":"i{i:02}","rater_id":"{rater}","source":"human","value":{},"version":1}}"#,
                    positive != flip
                ));
            }
        }
    }
    lines.join("\n") + "\n"
}

fn project(random: bool, split: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[project]\nname = \"detail\"\npositive = { equals = true }\ndecision = { score_at_least = 0.5 }\n[stats]\nseed = 1\n",
    )
    .unwrap();
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, records(random, split)).unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    dir
}

fn report(dir: &Path, args: &[&str]) -> Value {
    let out = evalsig(dir)
        .args(["--format", "json", "report", "s"])
        .args(args)
        .assert()
        .success();
    let v = serde_json::from_slice(&out.get_output().stdout).unwrap();
    common::check_results(&v);
    v
}

#[test]
fn score_runs_get_roc_and_calibration() {
    // Unit weights (no inclusion probabilities) and raters who agree: 0/1 references.
    let dir = project(false, false);
    let r = &report(dir.path(), &["--reference", "majority"])["results"];
    let roc = &r["roc"];
    // Items share clusters, so the variance is Obuchowski's.
    assert_eq!(roc["interval"], "obuchowski");
    let auc = roc["auc"].as_f64().unwrap();
    let (lo, hi) = (
        roc["ci"][0].as_f64().unwrap(),
        roc["ci"][1].as_f64().unwrap(),
    );
    assert!(lo < auc && auc < hi && auc > 0.8, "{roc}");
    // A run that separates the classes this well has a clearly positive d′.
    let sdt = &r["sdt"];
    assert_eq!(sdt["assumption"], "equal_variance_normal");
    assert!(sdt["dprime"].as_f64().unwrap() > 1.0, "{sdt}");
    assert!(sdt["criterion"].as_f64().unwrap().is_finite(), "{sdt}");
    assert_eq!(roc["points"][0], serde_json::json!([0.0, 0.0]));
    assert!(r["calibration"]["brier"].as_f64().unwrap() < 0.25);
    // Soft references (two raters who sometimes disagree) move the interval to the
    // bootstrap.
    let split = project(true, true);
    let soft = &report(split.path(), &["--reference", "soft"])["results"]["roc"];
    assert_eq!(soft["interval"], "bootstrap");
}

#[test]
fn features_report_likelihood_ratios() {
    let dir = project(true, true);
    let r = report(dir.path(), &["--features"]);
    let f = &r["results"]["features"][0];
    assert_eq!(f["feature_id"], "rule.a");
    assert!(f["lr"].as_f64().unwrap() > 1.0);
    assert!(f["class"].is_string());
    assert!(report(dir.path(), &[])["results"].get("features").is_none());
}

#[test]
fn reference_modes_are_reported_and_latent_is_refused() {
    let dir = project(true, true);
    for mode in ["single", "majority", "soft"] {
        let r = report(dir.path(), &["--reference", mode]);
        assert_eq!(r["results"]["reference"]["mode"], mode);
    }
    let out = evalsig(dir.path())
        .args(["--format", "json", "report", "s", "--reference", "latent"])
        .assert()
        .failure();
    let e: Value = serde_json::from_slice(&out.get_output().stderr).unwrap();
    assert!(e["error"]["message"].as_str().unwrap().contains("latent"));
}

#[test]
fn a_model_rater_corrects_the_human_scale_accuracy() {
    let dir = project(true, true);
    let r = report(
        dir.path(),
        &["--reference", "human", "--proxy", "model:judge"],
    );
    let p = &r["results"]["proxy"];
    assert_eq!(
        (p["labelled"].as_u64(), p["unlabelled"].as_u64()),
        (Some(30), Some(30))
    );
    assert_eq!(p["metric"], "accuracy");
    assert_eq!(p["corrected"], true);
    let (lo, hi) = (p["ci"][0].as_f64().unwrap(), p["ci"][1].as_f64().unwrap());
    let est = p["estimate"].as_f64().unwrap();
    assert!(lo < est && est < hi && p["lambda"].as_f64().unwrap() > 0.0);

    // Without inclusion probabilities the sample is not known to be random: no
    // correction.
    let plain = project(false, true);
    let p = &report(
        plain.path(),
        &["--reference", "human", "--proxy", "model:judge"],
    )["results"]["proxy"];
    assert_eq!(
        (p["corrected"].as_bool(), p["lambda"].as_f64()),
        (Some(false), Some(0.0))
    );

    for args in [
        &["--reference", "human"][..],
        &["--proxy", "model:judge"][..],
    ] {
        evalsig(dir.path())
            .args(["report", "s"])
            .args(args)
            .assert()
            .failure()
            .code(64);
    }
    let md = evalsig(dir.path())
        .args([
            "--format",
            "md",
            "report",
            "s",
            "--reference",
            "human",
            "--proxy",
            "model:judge",
            "--features",
        ])
        .assert()
        .success();
    let text = String::from_utf8(md.get_output().stdout.clone()).unwrap();
    assert!(
        text.contains("corrected with `model:judge`") && text.contains("rule.a"),
        "{text}"
    );
    assert!(text.contains("AUC "), "{text}");
}

#[test]
fn a_system_side_gap_enters_the_proxy_estimate_as_the_accuracy_worst_fill() {
    // 100 items carry a human label (half positive) and every item a model label equal
    // to the truth. The system is right everywhere, except that it times out on 20
    // negatives. The worst fill for accuracy on a negative is "judged positive", so
    // those 20 count as wrong: the human-scale accuracy is 80 / 100, whatever the
    // proxy says (the inclusion probabilities are unknown, so lambda is 0).
    let mut lines = vec![
        r#"{"record":"run","run_id":"s","system":{"id":"sys","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for i in 0..200 {
        let positive = i % 2 == 0;
        let timeout = i < 40 && !positive;
        lines.push(format!(r#"{{"record":"item","item_id":"i{i:03}"}}"#));
        lines.push(if timeout {
            format!(
                r#"{{"record":"judgment","item_id":"i{i:03}","run_id":"s","gen_replicate":0,"output_type":"binary","missing":{{"reason":"timeout"}}}}"#
            )
        } else {
            format!(
                r#"{{"record":"judgment","item_id":"i{i:03}","run_id":"s","gen_replicate":0,"output_type":"binary","value":{positive}}}"#
            )
        });
        lines.push(format!(
            r#"{{"record":"label","item_id":"i{i:03}","rater_id":"model:judge","source":"model","value":{positive},"version":1}}"#
        ));
        if i < 100 {
            lines.push(format!(
                r#"{{"record":"label","item_id":"i{i:03}","rater_id":"user:ana","source":"human","value":{positive},"version":1}}"#
            ));
        }
    }
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, lines.join("\n") + "\n").unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    let p = &report(
        dir.path(),
        &["--reference", "human", "--proxy", "model:judge"],
    )["results"]["proxy"];
    assert_eq!(p["lambda"].as_f64(), Some(0.0));
    assert!((p["estimate"].as_f64().unwrap() - 0.8).abs() < 1e-9, "{p}");
}
