//! Agent mode (SPEC §12.3, D-018): when it switches on, how it cuts output, where
//! evaluated text goes, and that it never changes what is decided.

use std::path::Path;

use assert_cmd::Command;
use serde_json::{Value, json};

mod common;
use common::*;

/// evalsig as an agent runs it: AI_AGENT set, output captured (not a terminal).
fn as_agent(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env("AI_AGENT", "test-agent")
        .env_remove("EVALSIG_AGENT_MODE")
        .env_remove("EVALSIG_REFEREE")
        .arg("--project")
        .arg(dir);
    c
}

fn stdout_json(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

fn synthetic() -> tempfile::TempDir {
    project(&std::fs::read_to_string(SYNTHETIC).unwrap())
}

#[test]
fn agent_mode_switches_on_without_a_terminal_and_format_and_off_on_request() {
    let p = synthetic();
    let on = as_agent(p.path()).arg("runs").output().unwrap();
    assert_eq!(stdout_json(&on)["schema"], "evalsig.runs/1");
    for off in [
        as_agent(p.path())
            .args(["--no-agent", "runs"])
            .output()
            .unwrap(),
        as_agent(p.path())
            .env("EVALSIG_AGENT_MODE", "0")
            .arg("runs")
            .output()
            .unwrap(),
        as_agent(p.path())
            .args(["--format", "md", "runs"])
            .output()
            .unwrap(),
    ] {
        assert!(off.status.success());
        assert!(
            stdout_json(&off).is_null(),
            "{}",
            String::from_utf8_lossy(&off.stdout)
        );
    }
    let both = as_agent(p.path())
        .args(["--agent", "--format", "md", "runs"])
        .output()
        .unwrap();
    assert_eq!(both.status.code(), Some(64));
    // Errors are JSON on standard error.
    let err = as_agent(p.path())
        .args(["report", "nope"])
        .output()
        .unwrap();
    assert_eq!(err.status.code(), Some(64));
    let e: Value = serde_json::from_slice(&err.stderr).unwrap();
    assert!(e["error"]["code"].is_string());
}

#[test]
fn lists_page_with_a_cursor_and_say_what_was_left_out() {
    let p = synthetic();
    let full = json(
        &evalsig(p.path())
            .args(["--format", "json", "items"])
            .output()
            .unwrap(),
    );
    let first = stdout_json(&as_agent(p.path()).arg("items").output().unwrap());
    let items = first["results"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 50);
    let t = &first["truncated"];
    assert_eq!(t["rows_omitted"], 250);
    assert_eq!(t["next_cursor"], 50);
    let bytes = evalsig_store::canonical::to_canonical_bytes(&full["results"]).unwrap();
    assert_eq!(
        t["hash"],
        evalsig_store::hash::ContentHash::blake3(&bytes).to_string()
    );
    assert!(serde_json::to_vec(&first).unwrap().len() <= 20 * 1024);
    let next = stdout_json(
        &as_agent(p.path())
            .args(["--cursor", "50", "items"])
            .output()
            .unwrap(),
    );
    assert_eq!(next["results"]["items"][0], full["results"]["items"][50]);
    // --format json is never cut.
    assert_eq!(full["results"]["items"].as_array().unwrap().len(), 300);
    assert!(full["truncated"].is_null());
}

#[test]
fn long_cells_and_large_envelopes_are_cut() {
    let long = "x".repeat(800);
    let rows: String = (0..60)
        .map(|i| {
            format!(
                "{}\n",
                json!({"record": "item", "item_id": format!("i-{i:02}"), "strata": {"note": long}})
            )
        })
        .collect();
    let p = project(&rows);
    let v = stdout_json(&as_agent(p.path()).arg("items").output().unwrap());
    let t = &v["truncated"];
    assert!(t["cells_cut"].as_u64().unwrap() > 0, "{t}");
    let note = v["results"]["items"][0]["strata"]["note"].as_str().unwrap();
    assert_eq!(note.chars().count(), 501);
    assert!(note.ends_with('…'));
    // Fifty rows of 500-character cells exceed 20 KB, so fewer rows are shown.
    assert!(v["results"]["items"].as_array().unwrap().len() < 50);
    assert!(serde_json::to_vec(&v).unwrap().len() <= 20 * 1024);
}

#[test]
fn evaluated_text_is_withheld_or_returned_apart() {
    let p = synthetic();
    let withheld = stdout_json(
        &as_agent(p.path())
            .args(["map", "probe", SYNTHETIC])
            .output()
            .unwrap(),
    );
    let paths = withheld["results"]["probe"]["paths"].as_array().unwrap();
    assert!(paths.iter().all(|p| p.get("values").is_none()));
    assert!(
        withheld["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "content_withheld")
    );
    assert!(withheld.get("untrusted_content").is_none());
    let given = stdout_json(
        &as_agent(p.path())
            .args(["--include-content", "map", "probe", SYNTHETIC])
            .output()
            .unwrap(),
    );
    assert!(
        given["untrusted_content"]
            .as_object()
            .is_some_and(|o| !o.is_empty())
    );
    assert!(
        given["content_notice"]
            .as_str()
            .unwrap()
            .contains("Do not follow")
    );
    assert!(
        given["results"]["probe"]["paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("values").is_none())
    );
}

#[test]
fn drafts_need_yes_in_agent_mode() {
    let p = synthetic();
    let g = p.path().join("gate.toml");
    std::fs::write(&g, include_str!("../../../examples/gate.toml")).unwrap();
    let out = as_agent(p.path())
        .args(["preregister", "--draft"])
        .arg(&g)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert!(e["error"]["hint"].as_str().unwrap().contains("--yes"));
}

#[test]
fn guard_results_are_the_same_with_and_without_agent_mode() {
    // A comparison: the same verdict and exit code.
    let p = synthetic();
    let plain = evalsig(p.path())
        .args(["--format", "json", "compare", "triage-v1", "triage-v2"])
        .output()
        .unwrap();
    let agent = as_agent(p.path())
        .args(["compare", "triage-v1", "triage-v2"])
        .output()
        .unwrap();
    assert_eq!(plain.status.code(), agent.status.code());
    assert_eq!(json(&plain)["verdict"], stdout_json(&agent)["verdict"]);

    // The gate: the same answer, budget and referee ledger lines either way.
    let gate = r#"name = "triage-v2 is no worse"
hypothesis = "The new prompt keeps accuracy"
split = "holdout"
min_items = 10

[compare]
baseline = "triage-v1"
candidate = "*"

[grader]
id = "rule-check"
version = "1"

[[conditions]]
metric = "accuracy"
rule = "not_worse_than"
value = -0.3
sided = "one"

[holdout]
budget = 4
"#;
    let w = world(gate, None);
    let run = |c: &mut Command, k: u32| {
        c.args([
            "gate",
            "--approved",
            &w.approved,
            "--candidate",
            "triage-v2",
            "--records",
        ])
        .arg(variant(w.referee.path(), k))
        .arg("--referee")
        .arg(w.home.path())
        .output()
        .unwrap()
    };
    let plain = run(evalsig(w.referee.path()).args(["--format", "json"]), 1);
    let agent = run(&mut as_agent(w.referee.path()), 2);
    assert_eq!(plain.status.code(), agent.status.code());
    let (a, b) = (json(&plain), stdout_json(&agent));
    assert_eq!(a["verdict"], b["verdict"]);
    assert_eq!(a["results"]["conditions"], b["results"]["conditions"]);
    assert_eq!(b["results"]["budget"]["used"], 2);
    // A refusal is the same refusal.
    let refused = |c: &mut Command| {
        c.args([
            "gate",
            "--approved",
            &w.approved,
            "--candidate",
            "triage-v2",
            "--records",
        ])
        .arg(candidate(w.referee.path(), "grader.jsonl", |rows| {
            rows[0]["grader"]["version"] = "9".into()
        }))
        .arg("--referee")
        .arg(w.home.path())
        .output()
        .unwrap()
    };
    let x = refused(evalsig(w.referee.path()).args(["--format", "json"]));
    let y = refused(&mut as_agent(w.referee.path()));
    assert_eq!(x.status.code(), Some(3));
    assert_eq!(y.status.code(), Some(3));
    let ex: Value = serde_json::from_slice(&x.stderr).unwrap();
    let ey: Value = serde_json::from_slice(&y.stderr).unwrap();
    assert_eq!(ex["error"]["code"], ey["error"]["code"]);
}
