//! `evalsig skill install|check` and `agent-guide` (agent-interface spec §3): files
//! generated from the binary, drift detected, and every command they name real.

use serde_json::Value;

mod common;
use common::{assert_command_real, evalsig_json as bin, json as out_json};

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/agent");

#[test]
fn an_installed_skill_is_checked_against_the_binary() {
    let dir = tempfile::tempdir().unwrap();
    let v = out_json(&bin(dir.path()).args(["skill", "install"]).output().unwrap());
    let skill = dir.path().join(".claude/skills/evalsig/SKILL.md");
    let text = std::fs::read_to_string(&skill).unwrap();
    assert!(text.contains(&format!("for evalsig {}", env!("CARGO_PKG_VERSION"))));
    assert!(!text.contains("{{version}}"));
    assert_eq!(v["results"]["files"].as_array().unwrap().len(), 3);
    assert!(
        bin(dir.path())
            .args(["skill", "check"])
            .assert()
            .success()
            .get_output()
            .status
            .success()
    );

    // An edited copy is detected, and install will not overwrite it without --force.
    std::fs::write(&skill, text.replace("Never", "Sometimes")).unwrap();
    let c = bin(dir.path()).args(["skill", "check"]).output().unwrap();
    assert_eq!(c.status.code(), Some(1));
    let v = out_json(&c);
    assert_eq!(v["verdict"]["code"], "fail");
    assert_eq!(v["warnings"][0]["code"], "skill_modified");
    assert_eq!(
        bin(dir.path())
            .args(["skill", "install"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(64)
    );
    bin(dir.path())
        .args(["skill", "install", "--force"])
        .assert()
        .success();
    bin(dir.path()).args(["skill", "check"]).assert().success();

    // A copy written by another version is reported as drift.
    let text = std::fs::read_to_string(&skill).unwrap();
    std::fs::write(
        &skill,
        text.replace(
            &format!("for evalsig {}", env!("CARGO_PKG_VERSION")),
            "for evalsig 0.0.1",
        ),
    )
    .unwrap();
    let v = out_json(&bin(dir.path()).args(["skill", "check"]).output().unwrap());
    assert_eq!(v["warnings"][0]["code"], "skill_version_drift");
    assert_eq!(v["results"]["installed_version"], "0.0.1");
    // A removed reference is missing.
    std::fs::remove_file(
        dir.path()
            .join(".claude/skills/evalsig/references/onboarding.md"),
    )
    .unwrap();
    let v = out_json(&bin(dir.path()).args(["skill", "check"]).output().unwrap());
    assert!(
        v["results"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["state"] == "missing")
    );
}

#[test]
fn targets_and_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
    bin(dir.path())
        .env("CODEX_HOME", codex.path())
        .args(["skill", "install", "--target", "codex", "--scope", "user"])
        .assert()
        .success();
    assert!(codex.path().join("skills/evalsig/SKILL.md").is_file());
    // No file a command writes lands in the store.
    let store = dir.path().join(".evalsig");
    let store = store.to_str().unwrap();
    for args in [
        vec!["skill", "install", "--target", "generic", "--dir", store],
        vec!["export", "--to", "csv", "--out", store],
    ] {
        assert_eq!(
            bin(dir.path()).args(&args).output().unwrap().status.code(),
            Some(64),
            "{args:?}"
        );
    }
    for args in [
        [
            "skill", "install", "--target", "codex", "--scope", "project",
        ],
        [
            "skill", "install", "--target", "generic", "--scope", "project",
        ],
    ] {
        assert_eq!(
            bin(dir.path()).args(args).output().unwrap().status.code(),
            Some(64)
        );
    }
}

#[test]
fn the_examples_are_what_the_binary_writes() {
    let tmp = tempfile::tempdir().unwrap();
    bin(tmp.path())
        .args(["skill", "check", "--target", "generic", "--dir"])
        .arg(format!("{EXAMPLES}/skills"))
        .assert()
        .success();
    for (flag, file) in [
        ("--snippet", "AGENTS.md.snippet"),
        ("--settings", "claude-settings.json"),
    ] {
        let out = bin(tmp.path())
            .args(["agent-guide", flag])
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            std::fs::read_to_string(format!("{EXAMPLES}/{file}")).unwrap(),
            "{file}: regenerate with evalsig agent-guide {flag}"
        );
    }
    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{EXAMPLES}/claude-settings.json")).unwrap(),
    )
    .unwrap();
    assert!(settings["permissions"]["deny"].as_array().unwrap().len() > 5);
}

/// Every `evalsig ...` command in backticks names real subcommands and flags.
#[test]
fn every_command_the_guides_name_exists() {
    let mut texts: Vec<(String, String)> = [
        "SKILL.md",
        "references/improve-loop.md",
        "references/onboarding.md",
    ]
    .iter()
    .map(|f| {
        (
            f.to_string(),
            std::fs::read_to_string(format!("{EXAMPLES}/skills/evalsig/{f}")).unwrap(),
        )
    })
    .collect();
    texts.push((
        "AGENTS.md.snippet".into(),
        std::fs::read_to_string(format!("{EXAMPLES}/AGENTS.md.snippet")).unwrap(),
    ));
    let mut checked = 0;
    for (file, text) in texts {
        for span in text.split('`').skip(1).step_by(2) {
            if let Some(cmd) = span.strip_prefix("evalsig ") {
                assert_command_real(cmd, &file);
                checked += 1;
            }
        }
    }
    assert!(checked > 30, "{checked}");
}
