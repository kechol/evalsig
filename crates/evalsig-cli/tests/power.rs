//! `evalsig power` and `evalsig threshold` against the sample-size table in
//! docs/sample-size.md and the cost references, through the CLI.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

const TABLE: &str = include_str!("../../../docs/sample-size.md");
const REFERENCE: &str = include_str!("../../../tests/fixtures/reference-values.json");

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .arg("--project")
        .arg(dir);
    c
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let out = evalsig(dir)
        .arg("--format")
        .arg("json")
        .args(args)
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

/// The rows of the table under a heading, as cells.
fn rows(section: &str) -> Vec<Vec<String>> {
    let start = TABLE.find(section).unwrap();
    TABLE[start..]
        .lines()
        .skip_while(|l| !l.starts_with("|---"))
        .skip(1)
        .take_while(|l| l.starts_with('|'))
        .map(|l| {
            l.trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_owned())
                .collect()
        })
        .collect()
}

#[test]
fn power_reproduces_the_sample_size_table() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for r in rows("## 1.") {
        let v = json(d, &["power", "--zero-events-below", &r[0]]);
        let z = &v["results"]["zero_events"];
        assert_eq!(
            (z["exact"].to_string(), z["wilson"].to_string()),
            (r[1].clone(), r[2].clone()),
            "{r:?}"
        );
    }
    let widths = ["0.10", "0.05", "0.03", "0.02"];
    for r in rows("## 2.") {
        for (w, want) in widths.iter().zip(&r[1..]) {
            let v = json(d, &["power", "--rate", &r[0], "--half-width", w]);
            assert_eq!(
                &v["results"]["rate_precision"]["items"].to_string(),
                want,
                "p={} w={w}",
                r[0]
            );
        }
    }
    for r in rows("## 3.") {
        let v = json(d, &["power", "--delta", &r[0], "--discordance", &r[1]]);
        let p = &v["results"]["paired"];
        assert_eq!(
            (p["exact"].to_string(), p["approximation"].to_string()),
            (r[2].clone(), r[3].clone()),
            "{r:?}"
        );
    }
    // metrics spec §12: one-sided, and at α/B for a budget of 20.
    let one = json(
        d,
        &[
            "power",
            "--delta",
            "0.05",
            "--discordance",
            "0.1",
            "--one-sided",
        ],
    );
    assert_eq!(one["results"]["paired"]["approximation"], 246);
    let gate = json(
        d,
        &[
            "power",
            "--delta",
            "0.05",
            "--discordance",
            "0.1",
            "--one-sided",
            "--budget",
            "20",
            "--n",
            "530",
        ],
    );
    assert_eq!(gate["results"]["paired"]["approximation"], 530);
    let pass = gate["results"]["paired"]["pass_probability"]
        .as_f64()
        .unwrap();
    assert!(pass > 0.7 && pass < 0.9, "{pass}");
    // The example gate: one-sided at 95% with a budget of 5.
    let example = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/gate.toml");
    let g = json(
        d,
        &[
            "power",
            "--gate",
            example,
            "--delta",
            "0.05",
            "--discordance",
            "0.1",
        ],
    );
    let b = json(
        d,
        &[
            "power",
            "--one-sided",
            "--budget",
            "5",
            "--delta",
            "0.05",
            "--discordance",
            "0.1",
        ],
    );
    assert_eq!(g["results"]["paired"], b["results"]["paired"]);
    assert_eq!(g["results"]["budget"], 5);
    let mdd = json(
        d,
        &["power", "--discordance", "0.1", "--n", "246", "--one-sided"],
    );
    assert!(mdd["results"]["paired"]["mde"].as_f64().unwrap() <= 0.05);
    let clustered = json(
        d,
        &[
            "power",
            "--rate",
            "0.5",
            "--half-width",
            "0.05",
            "--cluster-size",
            "3",
            "--icc",
            "0.5",
        ],
    );
    assert_eq!(clustered["results"]["rate_precision"]["items"], 762);
}

/// An ordinal run whose items reproduce positive and negative counts per level.
fn level_records(run: &str, pos: &[u64], neg: &[u64], prefix: &str) -> Vec<String> {
    let mut l = vec![format!(
        r#"{{"record":"run","run_id":"{run}","system":{{"id":"s","version":"{run}"}},"output_type":"ordinal","levels":4}}"#
    )];
    let mut k = 0;
    for (truth, counts) in [(true, pos), (false, neg)] {
        for (level, n) in counts.iter().enumerate() {
            for _ in 0..*n {
                let id = format!("{prefix}{k:03}");
                k += 1;
                l.push(format!(r#"{{"record":"item","item_id":"{id}"}}"#));
                l.push(format!(
                    r#"{{"record":"judgment","item_id":"{id}","run_id":"{run}","gen_replicate":0,"output_type":"ordinal","levels":4,"value":{level}}}"#
                ));
                l.push(format!(
                    r#"{{"record":"label","item_id":"{id}","rater_id":"ana","source":"human","value":{truth},"version":1}}"#
                ));
            }
        }
    }
    l
}

fn project(records: Vec<String>, config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), config).unwrap();
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, records.join("\n") + "\n").unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    dir
}

#[test]
fn threshold_matches_the_cost_references() {
    let refs: Value = serde_json::from_str(REFERENCE).unwrap();
    let c = &refs["cases"]["cost"][0];
    let counts = |k: &str| -> Vec<u64> {
        c["input"][k]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect()
    };
    let config = format!(
        "[project]\nname = \"cost\"\npositive = {{ equals = true }}\ndecision = {{ ordinal_at_least = 2 }}\n[cost]\nmatrix = {}\nprevalence = 0.12\nratio_range = [1, 20]\n",
        c["input"]["matrix"]
    );
    let dir = project(
        level_records("r", &counts("pos_by_level"), &counts("neg_by_level"), "i"),
        &config,
    );
    let v = json(dir.path(), &["threshold", "r", "--ratio", "5"]);
    let r = &v["results"];
    let e = &c["expected"];
    let close = |a: &Value, b: &Value| (a.as_f64().unwrap() - b.as_f64().unwrap()).abs() < 1e-9;
    assert!(close(&r["expected_cost"], &e["expected_cost"]));
    for (got, want) in r["threshold_costs"]
        .as_array()
        .unwrap()
        .iter()
        .zip(e["threshold_costs"].as_array().unwrap())
    {
        assert!(close(&got["expected_cost"], want));
    }
    assert_eq!(r["optimal_threshold"], e["optimal_threshold"]);
    assert_eq!(r["sensitivity"]["at_low"], e["optimal_at_low"]);
    assert_eq!(r["sensitivity"]["at_high"], e["optimal_at_high"]);
}

#[test]
fn threshold_compares_two_arms_over_the_cost_ratio_range() {
    let refs: Value = serde_json::from_str(REFERENCE).unwrap();
    let c = &refs["cases"]["cost"][1];
    // Ten positives and ten negatives each, at levels 0 and 3: H 0.8 / F 0.1 and
    // H 0.9 / F 0.2.
    let mut records = level_records("a", &[2, 0, 0, 8], &[9, 0, 0, 1], "a");
    records.extend(level_records("b", &[1, 0, 0, 9], &[8, 0, 0, 2], "b"));
    let config = "[project]\nname = \"cost\"\npositive = { equals = true }\ndecision = { ordinal_at_least = 2 }\n[cost]\nmatrix = [[0, 0, 1, 1], [5, 5, 0, 0]]\nprevalence = 0.12\nratio_range = [1, 20]\n";
    let dir = project(records, config);
    let v = json(dir.path(), &["threshold", "a", "--against", "b"]);
    let s = &v["results"]["difference"]["sensitivity"];
    let e = &c["expected"];
    assert!(
        (s["at_low"].as_f64().unwrap() - e["difference_at_low"].as_f64().unwrap()).abs() < 1e-9
    );
    assert!(
        (s["at_high"].as_f64().unwrap() - e["difference_at_high"].as_f64().unwrap()).abs() < 1e-9
    );
    assert_eq!(s["sign_changes"], e["sign_changes"]);
    // The matrix's corners give the default ratio C_FN / C_FP = 5 / 1.
    assert_eq!(v["results"]["ratio"], 5.0);
}

#[test]
fn an_indistinguishable_comparison_points_to_power() {
    // 60 items; run a is wrong on items 0–9, run b on items 5–15: accuracy 50/60
    // against 49/60; items 0–4 and 10–15 are right in one run only (11 of 60).
    let mut l = Vec::new();
    for run in ["a", "b"] {
        l.push(format!(
            r#"{{"record":"run","run_id":"{run}","system":{{"id":"s","version":"{run}"}},"output_type":"binary"}}"#
        ));
    }
    for i in 0..60 {
        let truth = i % 2 == 0;
        l.push(format!(r#"{{"record":"item","item_id":"i{i:02}"}}"#));
        l.push(format!(
            r#"{{"record":"label","item_id":"i{i:02}","rater_id":"ana","source":"human","value":{truth},"version":1}}"#
        ));
        for (run, wrong) in [("a", i < 10), ("b", (5..16).contains(&i))] {
            l.push(format!(
                r#"{{"record":"judgment","item_id":"i{i:02}","run_id":"{run}","gen_replicate":0,"output_type":"binary","value":{}}}"#,
                truth != wrong
            ));
        }
    }
    let dir = project(l, "[project]\nname = \"hint\"\n");
    let v = json(dir.path(), &["compare", "a", "b", "--scratch"]);
    assert_eq!(v["verdict"]["code"], "indistinguishable");
    let next = v["next"][0]["command"].as_str().unwrap();
    assert!(next.starts_with("evalsig power --delta "), "{next}");
    assert!(next.contains("--discordance 0.1833"), "{next}");
}
