//! Expected cost in `compare` (metrics spec §4, D-041): per stratum and overall against an
//! independent computation, the configured and the observed stratum mix, and the metric
//! as the primary one.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

const MATRIX: [[f64; 3]; 2] = [[0.0, 2.0, 1.0], [10.0, 2.0, 0.0]];
const S2_MATRIX: [[f64; 3]; 2] = [[0.0, 1.0, 1.0], [20.0, 3.0, 0.0]];

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("AI_AGENT")
        .arg("--project")
        .arg(dir);
    c
}

/// Item i: its stratum, reference, and each run's judged levels (two replicates for every
/// fifth item).
struct Item {
    stratum: &'static str,
    positive: bool,
    levels: [Vec<u8>; 2],
}

fn items() -> Vec<Item> {
    (0..80)
        .map(|i| {
            let positive = i % 3 == 0;
            let level = |shift: usize| ((i * 7 + shift) % 3) as u8;
            let run = |shift| {
                if i % 5 == 0 {
                    vec![level(shift), level(shift + 1)]
                } else if positive && shift == 1 {
                    // The candidate rejects more positives.
                    vec![2]
                } else {
                    vec![level(shift)]
                }
            };
            Item {
                stratum: if i < 30 { "S-1" } else { "S-2" },
                positive,
                levels: [run(0), run(1)],
            }
        })
        .collect()
}

fn records(items: &[Item]) -> String {
    let mut l = Vec::new();
    for run in ["base", "cand"] {
        l.push(format!(
            r#"{{"record":"run","run_id":"{run}","system":{{"id":"s","version":"{run}"}},"output_type":"ordinal","levels":3}}"#
        ));
    }
    for (i, it) in items.iter().enumerate() {
        l.push(format!(
            r#"{{"record":"item","item_id":"i{i:02}","strata":{{"section":"{}"}}}}"#,
            it.stratum
        ));
        l.push(format!(
            r#"{{"record":"label","item_id":"i{i:02}","rater_id":"ana","source":"human","value":{},"version":1}}"#,
            it.positive
        ));
        for (run, levels) in ["base", "cand"].iter().zip(&it.levels) {
            for (g, v) in levels.iter().enumerate() {
                l.push(format!(
                    r#"{{"record":"judgment","item_id":"i{i:02}","run_id":"{run}","gen_replicate":{g},"output_type":"ordinal","levels":3,"value":{v}}}"#
                ));
            }
        }
    }
    l.join("\n") + "\n"
}

fn project(cost: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let config = format!(
        "[project]\nname = \"ec\"\npositive = {{ equals = true }}\ndecision = {{ ordinal_at_least = 2 }}\n{cost}"
    );
    std::fs::write(dir.path().join(".evalsig/config.toml"), config).unwrap();
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, records(&items())).unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    dir
}

fn compare(dir: &Path, extra: &[&str]) -> Value {
    let out = evalsig(dir)
        .args(["--format", "json", "compare", "base", "cand"])
        .args(extra)
        .output()
        .unwrap();
    assert!(out.status.code().is_some_and(|c| c < 64), "{out:?}");
    let v = serde_json::from_slice(&out.stdout).unwrap();
    common::check_results(&v);
    v
}

/// The expected-cost row of a comparison.
fn expected_cost(v: &Value) -> &Value {
    v["results"]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric"] == "expected_cost")
        .unwrap()
}

/// EC per stratum and the arm's weighted item count per stratum, computed directly.
fn by_stratum(arm: usize, prevalence: &BTreeMap<&str, f64>) -> BTreeMap<&'static str, (f64, f64)> {
    let mut sums: BTreeMap<&str, [f64; 4]> = BTreeMap::new();
    for it in items() {
        let m = if it.stratum == "S-2" {
            S2_MATRIX
        } else {
            MATRIX
        };
        let row = usize::from(it.positive);
        let levels = &it.levels[arm];
        let cost = levels.iter().map(|l| m[row][*l as usize]).sum::<f64>() / levels.len() as f64;
        let t = sums.entry(it.stratum).or_default();
        t[row] += 1.0;
        t[2 + row] += cost;
    }
    sums.into_iter()
        .map(|(g, t)| {
            let pi = prevalence[g];
            (
                g,
                (pi * t[3] / t[1] + (1.0 - pi) * t[2] / t[0], t[0] + t[1]),
            )
        })
        .collect()
}

fn close(a: &Value, b: f64) {
    assert!((a.as_f64().unwrap() - b).abs() < 1e-6, "{a} != {b}");
}

const STRATA: &str = "[cost]\nmatrix = [[0, 2, 1], [10, 2, 0]]\nprevalence = 0.12\nby = \"section\"\n[cost.strata.\"S-1\"]\nprevalence = 0.29\nmix = 0.25\n[cost.strata.\"S-2\"]\nmatrix = [[0, 1, 1], [20, 3, 0]]\n";

#[test]
fn compare_gives_expected_cost_per_stratum_with_the_configured_mix() {
    let dir = project(&format!("{STRATA}mix = 0.75\n"));
    let v = compare(dir.path(), &["--metric", "expected_cost"]);
    let r = &v["results"];
    assert_eq!(r["primary_metric"], "expected_cost");
    let m = expected_cost(&v);
    let pi = BTreeMap::from([("S-1", 0.29), ("S-2", 0.12)]);
    let mix = BTreeMap::from([("S-1", 0.25), ("S-2", 0.75)]);
    let (a, b) = (by_stratum(0, &pi), by_stratum(1, &pi));
    let overall =
        |s: &BTreeMap<&str, (f64, f64)>| s.iter().map(|(g, (ec, _))| mix[g] * ec).sum::<f64>();
    close(&m["baseline"], overall(&a));
    close(&m["candidate"], overall(&b));
    close(&m["delta"], overall(&a) - overall(&b));
    assert_eq!(m["direction"], "lower_is_better");
    assert_eq!(m["mix"], "configured");
    assert_eq!(m["interval"], "bootstrap");
    let (lo, hi) = (m["ci"][0].as_f64().unwrap(), m["ci"][1].as_f64().unwrap());
    assert!(
        lo <= m["delta"].as_f64().unwrap() && m["delta"].as_f64().unwrap() <= hi,
        "{m}"
    );
    assert_eq!(v["verdict"]["code"], m["verdict"]);
    let strata = m["strata"].as_array().unwrap();
    assert_eq!(strata.len(), 2);
    for s in strata {
        let g = s["stratum"].as_str().unwrap();
        close(&s["prevalence"], pi[g]);
        close(&s["baseline"], a[g].0);
        close(&s["candidate"], b[g].0);
        assert!(s["ci"].is_array(), "{s}");
    }
    assert!(
        !v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "stratum_mix_observed"),
        "{v}"
    );
}

#[test]
fn a_stratum_without_a_mix_combines_strata_by_their_counts_and_warns() {
    let dir = project(STRATA);
    let v = compare(dir.path(), &[]);
    // The default primary metric stays the first rate.
    assert_eq!(v["results"]["primary_metric"], "accuracy");
    let m = expected_cost(&v);
    assert_eq!(m["mix"], "observed");
    let pi = BTreeMap::from([("S-1", 0.29), ("S-2", 0.12)]);
    let a = by_stratum(0, &pi);
    let n: f64 = a.values().map(|(_, n)| n).sum();
    close(&m["baseline"], a.values().map(|(ec, w)| ec * w / n).sum());
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "stratum_mix_observed"),
        "{v}"
    );
}

#[test]
fn without_strata_expected_cost_has_no_stratum_rows() {
    let dir = project("[cost]\nmatrix = [[0, 2, 1], [10, 2, 0]]\nprevalence = 0.12\n");
    let v = compare(dir.path(), &[]);
    let m = expected_cost(&v);
    assert!(m.get("strata").is_none() && m.get("mix").is_none(), "{m}");
    // One stratum: the matrix and prevalence of [cost], each S-2 item at the default
    // matrix too.
    let mut t = [0.0; 4];
    for it in items() {
        let row = usize::from(it.positive);
        let l = &it.levels[0];
        t[row] += 1.0;
        t[2 + row] += l.iter().map(|x| MATRIX[row][*x as usize]).sum::<f64>() / l.len() as f64;
    }
    close(&m["baseline"], 0.12 * t[3] / t[1] + 0.88 * t[2] / t[0]);
}

#[test]
fn without_cost_settings_expected_cost_is_not_a_metric() {
    let dir = project("");
    let out = evalsig(dir.path())
        .args([
            "--format",
            "json",
            "compare",
            "base",
            "cand",
            "--metric",
            "expected_cost",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "unknown_metric");
}

fn warned(v: &Value, code: &str) -> bool {
    v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["code"] == code)
}

#[test]
fn a_cost_stratum_no_item_has_is_named() {
    let dir = project(&STRATA.replace("\"S-1\"", "\"S1\""));
    let v = compare(dir.path(), &[]);
    assert!(warned(&v, "cost_stratum_unmatched"), "{v}");
    assert!(expected_cost(&v)["strata"].is_array());
}

#[test]
fn cost_settings_that_do_not_fit_the_levels_are_reported() {
    // Two columns, but the runs judge three levels.
    let dir = project("[cost]\nmatrix = [[0, 1], [5, 0]]\nprevalence = 0.12\n");
    let v = compare(dir.path(), &[]);
    assert!(warned(&v, "expected_cost_not_computed"), "{v}");
    assert!(
        !v["results"]["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["metric"] == "expected_cost")
    );
}

#[test]
fn report_gives_expected_cost_overall_and_per_stratum() {
    let dir = project(&format!("{STRATA}mix = 0.75\n"));
    let out = evalsig(dir.path())
        .args(["--format", "json", "report", "base"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let ec = &v["results"]["expected_cost"];
    let pi = BTreeMap::from([("S-1", 0.29), ("S-2", 0.12)]);
    let a = by_stratum(0, &pi);
    close(&ec["value"], 0.25 * a["S-1"].0 + 0.75 * a["S-2"].0);
    assert_eq!(ec["interval"], "bootstrap");
    assert_eq!(ec["n_used"], 80);
    assert_eq!(ec["mix"], "configured");
    let (lo, hi) = (ec["ci"][0].as_f64().unwrap(), ec["ci"][1].as_f64().unwrap());
    assert!(lo <= ec["value"].as_f64().unwrap() && ec["value"].as_f64().unwrap() <= hi);
    for s in ec["strata"].as_array().unwrap() {
        let g = s["stratum"].as_str().unwrap();
        close(&s["value"], a[g].0);
        close(&s["n"], a[g].1);
        assert!(s["ci"].is_array(), "{s}");
    }
    // The Markdown report shows it too.
    let out = evalsig(dir.path())
        .args(["--format", "md", "report", "base"])
        .output()
        .unwrap();
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(
        md.contains("Expected cost ") && md.contains("| S-1 | 0.2900 | 30.0 |"),
        "{md}"
    );
    // A run without [cost] has none, and says nothing about it.
    let dir = project("");
    let out = evalsig(dir.path())
        .args(["--format", "json", "report", "base"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["results"].get("expected_cost").is_none());
    assert!(!warned(&v, "expected_cost_not_computed"));
}
