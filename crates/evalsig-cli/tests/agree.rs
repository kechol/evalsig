//! `evalsig agree` on a multi-rater fixture (SPEC §10.1).

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

/// 40 items. `ana` gives the truth, `ben` flips every fifth item, `cy` every
/// seventh, the model every fourth; `ana` labels the first ten items again (wrong on
/// the third), and `lead` adjudicates item 0. A run decides right except every sixth
/// item. Preferences compare runs a, b and c (both orders for a–b) when `connected`,
/// or a–b and c–d only otherwise.
fn records(connected: bool) -> String {
    let mut l = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    let label = |item: usize, rater: &str, source: &str, v: bool, extra: &str| {
        format!(
            r#"{{"record":"label","item_id":"i{item:02}","rater_id":"{rater}","source":"{source}","value":{v},"version":1{extra}}}"#
        )
    };
    for i in 0..40 {
        let truth = i % 3 == 0;
        l.push(format!(r#"{{"record":"item","item_id":"i{i:02}"}}"#));
        l.push(format!(
            r#"{{"record":"judgment","item_id":"i{i:02}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{}}}"#,
            truth != (i % 6 == 1)
        ));
        l.push(label(i, "ana", "human", truth, ""));
        l.push(label(i, "ben", "human", truth != (i % 5 == 0), ""));
        l.push(label(i, "cy", "human", truth != (i % 7 == 0), ""));
        l.push(label(i, "model:judge", "model", truth != (i % 4 == 0), ""));
        if i < 10 {
            l.push(label(
                i,
                "ana",
                "human",
                truth != (i == 3),
                r#","label_replicate":1"#,
            ));
        }
    }
    l.push(label(
        0,
        "lead",
        "human",
        false,
        r#","kind":"adjudication""#,
    ));
    let pref = |item: usize, left: &str, right: &str, rater: &str, outcome: &str| {
        format!(
            r#"{{"record":"preference","item_id":"i{item:02}","left_run_id":"{left}","right_run_id":"{right}","rater_id":"{rater}","source":"human","outcome":"{outcome}"}}"#
        )
    };
    for i in 0..12 {
        for rater in ["ana", "ben"] {
            l.push(pref(
                i,
                "a",
                "b",
                rater,
                if i % 4 == 0 { "left" } else { "right" },
            ));
            l.push(pref(
                i,
                "b",
                "a",
                rater,
                if i % 4 == 0 { "right" } else { "left" },
            ));
            if connected {
                l.push(pref(
                    i,
                    "b",
                    "c",
                    rater,
                    if i % 3 == 0 { "tie" } else { "right" },
                ));
            } else {
                l.push(pref(i, "c", "d", rater, "left"));
            }
        }
    }
    l.join("\n") + "\n"
}

fn project(connected: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, records(connected)).unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    dir
}

fn agree(dir: &Path, args: &[&str]) -> Value {
    let out = evalsig(dir)
        .args(["--format", "json", "agree"])
        .args(args)
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

/// Cohen's κ of two 0/1 lists by hand.
fn kappa(a: &[bool], b: &[bool]) -> f64 {
    let n = a.len() as f64;
    let po = a.iter().zip(b).filter(|(x, y)| x == y).count() as f64 / n;
    let (pa, pb) = (
        a.iter().filter(|x| **x).count() as f64 / n,
        b.iter().filter(|x| **x).count() as f64 / n,
    );
    let pe = pa * pb + (1.0 - pa) * (1.0 - pb);
    (po - pe) / (1.0 - pe)
}

#[test]
fn raters_graders_and_the_run_are_compared_under_the_same_rule() {
    let dir = project(true);
    let v = agree(dir.path(), &["--run", "r"]);
    let r = &v["results"];
    // Four raters (the adjudicator is not one), six pairs.
    let raters: Vec<&str> = r["raters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["rater"].as_str().unwrap())
        .collect();
    assert_eq!(raters, ["ana", "ben", "cy", "model:judge"]);
    assert_eq!(r["pairs"].as_array().unwrap().len(), 6);

    let truth: Vec<bool> = (0..40).map(|i| i % 3 == 0).collect();
    let ben: Vec<bool> = (0..40).map(|i| truth[i] != (i % 5 == 0)).collect();
    let ab = r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["raters"] == serde_json::json!(["ana", "ben"]))
        .unwrap();
    let got = ab["decisions"]["cohen_kappa"].as_f64().unwrap();
    assert!((got - kappa(&truth, &ben)).abs() < 1e-9, "{got}");

    let same = |who: &str| {
        r["same_rule"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["who"] == who)
            .unwrap()
            .clone()
    };
    // Each human is compared with the other two; the grader and the run with all three.
    assert_eq!(same("ana")["humans_compared"], 2);
    assert_eq!(same("model:judge")["humans_compared"], 3);
    assert_eq!(same("r")["kind"], "system");
    assert_eq!(same("r")["humans_compared"], 3);
    let cy: Vec<bool> = (0..40).map(|i| truth[i] != (i % 7 == 0)).collect();
    let system: Vec<bool> = (0..40).map(|i| truth[i] != (i % 6 == 1)).collect();
    let expected = (kappa(&system, &truth) + kappa(&system, &ben) + kappa(&system, &cy)) / 3.0;
    assert!((same("r")["mean_kappa"].as_f64().unwrap() - expected).abs() < 1e-9);

    // ana relabelled ten items and changed one.
    let var = &r["variability"]["default"];
    assert_eq!(var["within_rater"]["pairs"], 10);
    assert_eq!(var["within_rater"]["disagreeing"], 1);
    assert!(r["ceiling"].as_f64().unwrap() > 0.5);
}

#[test]
fn preferences_give_rater_agreement_position_bias_and_a_ranking() {
    let dir = project(true);
    let p = &agree(dir.path(), &["--preference"])["results"]["preference"];
    let ab = &p["comparisons"][0];
    assert_eq!(ab["arms"], serde_json::json!(["a", "b"]));
    assert_eq!(ab["rater_agreement"][0]["cohen_kappa"], 1.0);
    assert_eq!(ab["position_bias_by_rater"]["ana"]["swapped_pairs"], 12);
    let order: Vec<&str> = p["ranking"]["arms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["arm"].as_str().unwrap())
        .collect();
    assert_eq!(order, ["c", "b", "a"]);

    let split = project(false);
    let v = agree(split.path(), &["--preference"]);
    assert!(v["results"]["preference"]["ranking"].is_null());
    let codes: Vec<&str> = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"comparison_graph_disconnected"),
        "{codes:?}"
    );
}

#[test]
fn the_latent_reference_is_refused() {
    let dir = project(true);
    evalsig(dir.path())
        .args(["agree", "--reference", "latent"])
        .assert()
        .failure();
}
