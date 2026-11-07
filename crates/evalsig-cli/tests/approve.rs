//! `evalsig preregister approve` (SPEC §8.3): the referee resolves and pins a draft,
//! and the approved conditions read back the same whatever the configuration says.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");

const GATE: &str = r#"name = "triage-v2 is more accurate"
hypothesis = "The new prompt raises accuracy by 5 points"
split = "holdout"
min_items = 40

[compare]
baseline = "triage-v1"
candidate = "*"

[grader]
id = "rubric-judge"
version = "j-1"

[[conditions]]
metric = "accuracy"
rule = "better_by"
value = 0.0
min_effect = 0.05
expected_effect = 0.08
sided = "one"

[holdout]
budget = 5
"#;

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("EVALSIG_REFEREE")
        .arg("--project")
        .arg(dir);
    c
}

fn json(out: &std::process::Output) -> Value {
    serde_json::from_slice(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    })
    .unwrap()
}

fn project(records: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    let f = dir.path().join("records.jsonl");
    std::fs::write(&f, records).unwrap();
    evalsig(dir.path()).arg("import").arg(&f).assert().success();
    dir
}

struct World {
    referee_project: tempfile::TempDir,
    home: tempfile::TempDir,
    agent: tempfile::TempDir,
    holdout: String,
    draft: String,
}

fn world() -> World {
    let all = std::fs::read_to_string(SYNTHETIC).unwrap();
    let referee_project = project(&all);
    let home = tempfile::tempdir().unwrap();
    evalsig(referee_project.path())
        .args(["referee", "init"])
        .arg(home.path())
        .assert()
        .success();
    let remaining = referee_project.path().join("remaining.json");
    let out = evalsig(referee_project.path())
        .args(["--format", "json", "split", "--holdout", "--referee"])
        .arg(home.path())
        .arg("--out")
        .arg(&remaining)
        .output()
        .unwrap();
    let holdout = json(&out)["results"]["holdout"]["hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(&remaining).unwrap()).unwrap();
    let kept: Vec<&str> = kept["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap())
        .collect();
    let agent_records: String = all
        .lines()
        .filter(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["record"] != "label" || kept.contains(&v["item_id"].as_str().unwrap())
        })
        .map(|l| format!("{l}\n"))
        .collect();
    let agent = project(&agent_records);
    evalsig(referee_project.path())
        .args(["seal", "--holdout", &holdout, "--referee"])
        .arg(home.path())
        .arg("--agent-project")
        .arg(agent.path())
        .assert()
        .success();
    let gate = agent.path().join("gate.toml");
    std::fs::write(&gate, GATE).unwrap();
    let out = evalsig(agent.path())
        .args(["--format", "json", "preregister", "--draft"])
        .arg(&gate)
        .output()
        .unwrap();
    let draft = json(&out)["results"]["draft"].as_str().unwrap().to_owned();
    World {
        referee_project,
        home,
        agent,
        holdout,
        draft,
    }
}

fn approve(w: &World, referee: Option<&Path>) -> std::process::Output {
    let mut c = evalsig(w.referee_project.path());
    c.args([
        "--format",
        "json",
        "preregister",
        "approve",
        &w.draft,
        "--holdout",
        &w.holdout,
        "--from",
    ])
    .arg(w.agent.path());
    if let Some(r) = referee {
        c.arg("--referee").arg(r);
    }
    c.output().unwrap()
}

#[test]
fn approval_resolves_and_pins_and_ignores_later_configuration() {
    let w = world();
    let out = approve(&w, Some(w.home.path()));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out);
    let r = &v["results"]["resolved"];
    for key in [
        "positive",
        "estimand",
        "missing",
        "per_submission_level",
        "bootstrap",
        "reference_mode",
        "output_type",
        "value_semantics",
        "baseline_snapshot",
        "split_entry",
        "seal_entry",
        "holdout_items_hash",
        "seed_derivation",
    ] {
        assert!(!r[key].is_null(), "{key} unresolved: {r}");
    }
    assert!((r["per_submission_level"].as_f64().unwrap() - 0.99).abs() < 1e-12);
    // 50 / (α/B) = 5000 resamples at least.
    assert!(r["bootstrap"].as_u64().unwrap() >= 5000);
    assert!(
        v["results"]["power"][0]["pass_probability"]
            .as_f64()
            .is_some(),
        "{v}"
    );
    assert!(v["results"]["label_provenance"]["human"].as_u64().is_some());
    let approved = v["results"]["approved"].as_str().unwrap().to_owned();
    let ledger = std::fs::read_to_string(w.home.path().join("ledger.jsonl")).unwrap();
    assert!(
        ledger
            .lines()
            .any(|l| l.contains("\"preregister_approve\"") && l.contains(&approved))
    );

    // gate reads the approved copy only: rewriting the configuration changes nothing.
    let show = |p: &Path| {
        let out = evalsig(p)
            .args([
                "--format",
                "json",
                "preregister",
                "show",
                &approved,
                "--referee",
            ])
            .arg(w.home.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        json(&out)["results"]["conditions"].clone()
    };
    let before = show(w.referee_project.path());
    std::fs::write(
        w.referee_project.path().join(".evalsig/config.toml"),
        "[project]\nname = \"rewritten\"\npositive = { equals = false }\n[stats]\nlevel = 0.5\nseed = 1\n",
    )
    .unwrap();
    std::fs::remove_file(w.agent.path().join(".evalsig/config.toml")).unwrap();
    assert_eq!(show(w.referee_project.path()), before);

    // A copy edited after approval no longer matches its hash.
    let file = w.home.path().join("approved").join(format!(
        "{}.toml",
        approved.strip_prefix("blake3:").unwrap()
    ));
    let text = std::fs::read_to_string(&file)
        .unwrap()
        .replace("min_items = 40", "min_items = 4");
    std::fs::write(&file, text).unwrap();
    evalsig(w.referee_project.path())
        .args(["preregister", "show", &approved, "--referee"])
        .arg(w.home.path())
        .assert()
        .failure()
        .code(3);
}

#[test]
fn approval_needs_the_referee_a_seal_and_enough_items() {
    let w = world();
    let out = approve(&w, None);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(json(&out)["error"]["code"], "referee_only");

    // Too few holdout items for min_items.
    let big = GATE.replace("min_items = 40", "min_items = 5000");
    let gate = w.agent.path().join("big.toml");
    std::fs::write(&gate, big).unwrap();
    let d = json(
        &evalsig(w.agent.path())
            .args(["--format", "json", "preregister", "--draft"])
            .arg(&gate)
            .output()
            .unwrap(),
    );
    let out = evalsig(w.referee_project.path())
        .args([
            "--format",
            "json",
            "preregister",
            "approve",
            d["results"]["draft"].as_str().unwrap(),
            "--holdout",
            &w.holdout,
            "--from",
        ])
        .arg(w.agent.path())
        .arg("--referee")
        .arg(w.home.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn approval_fixes_the_ladder_step_and_refuses_a_finer_one() {
    let w = world();
    let out = approve(&w, Some(w.home.path()));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ladder = &json(&out)["results"]["resolved"]["ladder"];
    assert_eq!(ladder["reuse_guard"], "ladder");
    assert_eq!(ladder["metric"], "accuracy");
    assert_eq!(ladder["k"], 2.0);
    // Accuracy's denominator is every holdout item.
    let remaining: Value = serde_json::from_str(
        &std::fs::read_to_string(w.referee_project.path().join("remaining.json")).unwrap(),
    )
    .unwrap();
    let n = 300.0 - remaining["items"].as_array().unwrap().len() as f64;
    assert_eq!(ladder["denominator"].as_f64().unwrap(), n);
    let psi = ladder["psi_hat"].as_f64().unwrap();
    assert!(psi >= 0.05_f64.max(4.0 / n) && psi <= 1.0, "{ladder}");
    let step = ladder["step"].as_f64().unwrap();
    assert!(
        (step - (2.0 * (psi / n).sqrt()).max(1.0 / n)).abs() < 1e-9,
        "{ladder}"
    );
    let best = ladder["first_best"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&best));

    // A step below the automatic one returns more than the holdout can tell.
    let gate = w.agent.path().join("fine.toml");
    std::fs::write(
        &gate,
        GATE.replace("budget = 5", "budget = 5\nladder_step = 0.001"),
    )
    .unwrap();
    let out = evalsig(w.agent.path())
        .args(["--format", "json", "preregister", "--draft"])
        .arg(&gate)
        .output()
        .unwrap();
    let draft = json(&out)["results"]["draft"].as_str().unwrap().to_owned();
    let out = evalsig(w.referee_project.path())
        .args([
            "--format",
            "json",
            "preregister",
            "approve",
            &draft,
            "--holdout",
            &w.holdout,
            "--from",
        ])
        .arg(w.agent.path())
        .arg("--referee")
        .arg(w.home.path())
        .output()
        .unwrap();
    // D-015: a usage error.
    assert_eq!(
        out.status.code(),
        Some(64),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        json(&out)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("below the automatic step")
    );
}
