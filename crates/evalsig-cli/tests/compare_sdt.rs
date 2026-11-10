//! `compare --sdt` (D-042): ΔH, ΔF, Δd′ and Δc with paired intervals, and the change
//! classification for each case.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("AI_AGENT")
        .arg("--project")
        .arg(dir);
    c
}

/// `positives` and `negatives` items; each run calls the first `round(h · positives)`
/// positives and the first `round(f · negatives)` negatives positive.
fn project(positives: usize, negatives: usize, runs: &[(&str, f64, f64)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[project]\nname = \"sdt\"\npositive = { equals = true }\ndecision = { equals = true }\n",
    )
    .unwrap();
    let mut l = Vec::new();
    for (run, _, _) in runs {
        l.push(format!(
            r#"{{"record":"run","run_id":"{run}","system":{{"id":"s","version":"{run}"}},"output_type":"binary"}}"#
        ));
    }
    for i in 0..positives + negatives {
        let positive = i < positives;
        l.push(format!(r#"{{"record":"item","item_id":"i{i:03}"}}"#));
        l.push(format!(
            r#"{{"record":"label","item_id":"i{i:03}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
        ));
        for (run, h, f) in runs {
            let (rank, n, rate) = if positive {
                (i, positives, h)
            } else {
                (i - positives, negatives, f)
            };
            let judged = (rank as f64) < (rate * n as f64).round();
            l.push(format!(
                r#"{{"record":"judgment","item_id":"i{i:03}","run_id":"{run}","gen_replicate":0,"output_type":"binary","value":{judged}}}"#
            ));
        }
    }
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, l.join("\n") + "\n").unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    dir
}

fn sdt(dir: &Path, base: &str, cand: &str, extra: &[&str]) -> Value {
    let out = evalsig(dir)
        .args([
            "--format",
            "json",
            "compare",
            base,
            cand,
            "--sdt",
            "--scratch",
        ])
        .args(extra)
        .output()
        .unwrap();
    assert!(out.status.code().is_some_and(|c| c < 64), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["results"]["sdt"].clone()
}

fn delta<'a>(s: &'a Value, metric: &str) -> &'a Value {
    s["deltas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["metric"] == metric)
        .unwrap()
}

#[test]
fn each_change_is_classified_from_the_intervals() {
    let dir = project(
        120,
        120,
        &[
            ("weak", 0.70, 0.30),
            ("sharp", 0.90, 0.10),
            // About the same d′ as weak at a more liberal criterion.
            ("liberal", 0.85, 0.48),
            ("twin", 0.71, 0.30),
        ],
    );
    let d = dir.path();
    assert_eq!(
        sdt(d, "weak", "sharp", &[])["change"],
        "discrimination_improved"
    );
    assert_eq!(
        sdt(d, "sharp", "weak", &[])["change"],
        "discrimination_worsened"
    );
    assert_eq!(sdt(d, "weak", "liberal", &[])["change"], "threshold_moved");
    assert_eq!(sdt(d, "weak", "twin", &[])["change"], "undetermined");
    // A minimum above the smallest cell withholds the classification.
    let s = sdt(d, "weak", "sharp", &["--min-events", "50"]);
    assert_eq!(s["change"], "insufficient_data");
    assert_eq!(s["min_events"], 50.0);
    assert!(delta(&s, "dprime")["ci"].is_array(), "{s}");
}

#[test]
fn the_deltas_match_each_runs_report() {
    let dir = project(80, 80, &[("weak", 0.70, 0.30), ("sharp", 0.90, 0.10)]);
    let s = sdt(dir.path(), "weak", "sharp", &[]);
    assert_eq!(s["assumption"], "equal_variance_normal");
    let report = |run: &str| -> Value {
        let out = evalsig(dir.path())
            .args(["--format", "json", "report", run])
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let close = |a: &Value, b: &Value| {
        assert!(
            (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-3,
            "{a} {b}"
        )
    };
    for (run, side) in [("weak", "baseline"), ("sharp", "candidate")] {
        let r = report(run);
        close(&delta(&s, "dprime")[side], &r["results"]["sdt"]["dprime"]);
        close(
            &delta(&s, "criterion")[side],
            &r["results"]["sdt"]["criterion"],
        );
    }
    close(&delta(&s, "hit_rate")["delta"], &serde_json::json!(0.2));
    close(
        &delta(&s, "false_alarm_rate")["delta"],
        &serde_json::json!(-0.2),
    );
    let dp = delta(&s, "dprime");
    let (lo, hi) = (dp["ci"][0].as_f64().unwrap(), dp["ci"][1].as_f64().unwrap());
    assert!(lo <= dp["delta"].as_f64().unwrap() && dp["delta"].as_f64().unwrap() <= hi);
}

#[test]
fn sdt_needs_paired_arms_and_is_absent_unless_asked() {
    let dir = project(20, 20, &[("a", 0.7, 0.3), ("b", 0.8, 0.2)]);
    let out = evalsig(dir.path())
        .args([
            "--format",
            "json",
            "compare",
            "a",
            "b",
            "--sdt",
            "--unpaired",
        ])
        .output()
        .unwrap();
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "conflicting_arguments");
    // Without --sdt, nothing is added.
    let out = evalsig(dir.path())
        .args(["--format", "json", "compare", "a", "b", "--scratch"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["results"].get("sdt").is_none());
}

#[test]
fn explain_knows_every_change_class() {
    let dir = tempfile::tempdir().unwrap();
    for class in [
        "discrimination_improved",
        "discrimination_worsened",
        "threshold_moved",
        "undetermined",
    ] {
        let out = evalsig(dir.path())
            .args(["--format", "json", "explain", class])
            .output()
            .unwrap();
        assert!(out.status.success(), "{class}: {out:?}");
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(
            !v["results"]["matches"].as_array().unwrap().is_empty(),
            "{class}"
        );
    }
}

#[test]
fn markdown_shows_the_deltas_and_bad_minimums_are_refused() {
    let dir = project(80, 80, &[("weak", 0.70, 0.30), ("sharp", 0.90, 0.10)]);
    let out = evalsig(dir.path())
        .args([
            "--format",
            "md",
            "compare",
            "weak",
            "sharp",
            "--sdt",
            "--scratch",
        ])
        .output()
        .unwrap();
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(
        md.contains("| dprime |") && md.contains("Change: `discrimination_improved`"),
        "{md}"
    );
    for bad in ["nan", "-1", "inf"] {
        let out = evalsig(dir.path())
            .args(["--format", "json", "compare", "weak", "sharp", "--sdt"])
            .args(["--min-events", bad])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64), "{bad}");
    }
}
