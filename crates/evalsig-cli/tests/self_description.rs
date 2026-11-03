//! The self-description commands (agent-interface spec §2.6, §3.3): `help --json`
//! from the clap definitions, `explain`, the envelope schema and `recipes`.

use std::collections::BTreeSet;

use assert_cmd::Command;
use serde_json::Value;

mod common;
use common::*;

fn bin() -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env_remove("AI_AGENT").env("EVALSIG_ACTOR", "user:test");
    c
}

fn help_json() -> Value {
    let out = bin().args(["help", "--json"]).output().unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Subcommand names in the "Commands:" section of clap's own --help text.
fn listed(args: &[&str]) -> BTreeSet<String> {
    let out = bin().args(args).arg("--help").output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    text.lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| *n != "help")
        .map(str::to_owned)
        .collect()
}

#[test]
fn help_json_lists_every_command_clap_defines() {
    let h = help_json();
    let commands = h["results"]["commands"].as_array().unwrap();
    let names: BTreeSet<String> = commands
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, listed(&[]));
    for c in commands {
        let name = c["name"].as_str().unwrap();
        let subs: BTreeSet<String> = c["subcommands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(subs, listed(&[name]), "{name}");
    }
    // Arguments, exit codes and verdict codes come along.
    let compare = commands.iter().find(|c| c["name"] == "compare").unwrap();
    assert!(
        compare["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["long"] == "split")
    );
    assert_eq!(h["results"]["exit_codes"].as_array().unwrap().len(), 7);
    assert!(
        h["results"]["verdict_codes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["meaning"].as_str().is_some_and(|m| !m.is_empty()))
    );
}

#[test]
fn explain_covers_terms_verdicts_checks_and_exit_codes() {
    let explain = |name: &str| {
        let out = bin()
            .args(["--format", "json", "explain", name])
            .output()
            .unwrap();
        (out.status.code(), json(&out))
    };
    for (name, kind) in [
        ("hit_rate", "term"),
        ("accuracy", "term"),
        ("indistinguishable", "verdict"),
        ("identical_replicates", "validity_check"),
        ("3", "exit_code"),
    ] {
        let (code, v) = explain(name);
        assert_eq!(code, Some(0), "{name}");
        assert!(
            v["results"]["matches"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["kind"] == kind),
            "{name}: {v}"
        );
    }
    let (code, v) = explain("no_such_thing");
    assert_eq!(code, Some(64));
    assert!(v["error"]["hint"].as_str().unwrap().contains("help --json"));
}

fn validator(defs_error: bool) -> jsonschema::Validator {
    let out = bin()
        .args(["schema", "envelope", "--raw"])
        .output()
        .unwrap();
    let mut schema: Value = serde_json::from_slice(&out.stdout).unwrap();
    if defs_error {
        schema["$ref"] = "#/$defs/error".into();
        for k in ["type", "required", "properties", "additionalProperties"] {
            schema.as_object_mut().unwrap().remove(k);
        }
    }
    jsonschema::validator_for(&schema).unwrap()
}

#[test]
fn every_output_matches_the_envelope_schema() {
    let p = project(&std::fs::read_to_string(SYNTHETIC).unwrap());
    let v = validator(false);
    let run = |args: &[&str], agent: bool| -> Value {
        let mut c = bin();
        if agent {
            c.env("AI_AGENT", "test-agent");
        } else {
            c.args(["--format", "json"]);
        }
        let out = c
            .arg("--project")
            .arg(p.path())
            .args(args)
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let outputs = [
        run(&["version"], false),
        run(&["runs"], false),
        run(&["report", "triage-v1"], false),
        run(&["compare", "triage-v1", "triage-v2"], false),
        run(&["explain", "accuracy"], false),
        run(&["recipes"], false),
        run(&["items"], true),
        run(&["--include-content", "map", "probe", SYNTHETIC], true),
        help_json(),
    ];
    for o in &outputs {
        let errors: Vec<String> = v.iter_errors(o).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{}: {errors:?}", o["schema"]);
    }
    assert!(outputs[6]["truncated"].is_object());
    let e = bin()
        .args(["--format", "json", "--project"])
        .arg(p.path())
        .args(["report", "nope"])
        .output()
        .unwrap();
    let err: Value = serde_json::from_slice(&e.stderr).unwrap();
    assert!(validator(true).is_valid(&err), "{err}");
}

#[test]
fn every_flag_a_recipe_names_exists() {
    let list = json(
        &bin()
            .args(["--format", "json", "recipes"])
            .output()
            .unwrap(),
    );
    let names: Vec<String> = list["results"]["recipes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["recipe"].as_str().unwrap().to_owned())
        .filter(|n| !n.contains(' '))
        .collect();
    assert_eq!(names.len(), 5);
    for name in names {
        let r = json(
            &bin()
                .args(["--format", "json", "recipes", &name])
                .output()
                .unwrap(),
        );
        for step in r["results"]["steps"].as_array().unwrap() {
            for cmd in step.as_str().unwrap().split("evalsig ").skip(1) {
                assert_command_real(cmd, &name);
            }
        }
    }
}
