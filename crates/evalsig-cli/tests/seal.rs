//! `evalsig seal` (SPEC §8.2, D-013): sealed files open with the standard age CLI,
//! sealing refuses when the agent already holds holdout labels, and resealing after a
//! revision leaves only hashes in the referee ledger.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command as Process;

use assert_cmd::Command;
use serde_json::Value;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("EVALSIG_REFEREE")
        .arg("--project")
        .arg(dir);
    c
}

fn project(records: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, records).unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    dir
}

struct Setup {
    referee_project: tempfile::TempDir,
    home: tempfile::TempDir,
    holdout: String,
    held: BTreeSet<String>,
}

fn setup() -> Setup {
    let referee_project = project(&std::fs::read_to_string(SYNTHETIC).unwrap());
    let home = tempfile::tempdir().unwrap();
    evalsig(referee_project.path())
        .args(["referee", "init"])
        .arg(home.path())
        .assert()
        .success();
    let out = evalsig(referee_project.path())
        .args(["--format", "json", "split", "--holdout", "--referee"])
        .arg(home.path())
        .arg("--out")
        .arg(referee_project.path().join("remaining.json"))
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let holdout = v["results"]["holdout"]["hash"].as_str().unwrap().to_owned();
    let remaining: Value = serde_json::from_str(
        &std::fs::read_to_string(referee_project.path().join("remaining.json")).unwrap(),
    )
    .unwrap();
    let kept: BTreeSet<String> = remaining["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap().to_owned())
        .collect();
    let all: BTreeSet<String> = std::fs::read_to_string(SYNTHETIC)
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["record"] == "item")
        .map(|v| v["item_id"].as_str().unwrap().to_owned())
        .collect();
    let held = all.difference(&kept).cloned().collect();
    Setup {
        referee_project,
        home,
        holdout,
        held,
    }
}

/// The synthetic records the agent may have: no label of a holdout item.
fn agent_records(held: &BTreeSet<String>) -> String {
    std::fs::read_to_string(SYNTHETIC)
        .unwrap()
        .lines()
        .filter(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            !(v["record"] == "label" && held.contains(v["item_id"].as_str().unwrap()))
        })
        .map(|l| format!("{l}\n"))
        .collect()
}

fn seal(s: &Setup, agent: &Path) -> std::process::Output {
    evalsig(s.referee_project.path())
        .args([
            "--format",
            "json",
            "seal",
            "--holdout",
            &s.holdout,
            "--referee",
        ])
        .arg(s.home.path())
        .arg("--agent-project")
        .arg(agent)
        .output()
        .unwrap()
}

#[test]
fn sealed_labels_open_with_the_age_cli() {
    let s = setup();
    let agent = project(&agent_records(&s.held));
    let out = seal(&s, agent.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let file = v["results"]["file"].as_str().unwrap();
    let plain = match Process::new("age")
        .arg("-d")
        .arg("-i")
        .arg(s.home.path().join("identity.txt"))
        .arg(file)
        .output()
    {
        Ok(o) => {
            assert!(
                o.status.success(),
                "age -d refused the sealed file: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            o.stdout
        }
        // CI installs age and sets EVALSIG_REQUIRE_AGE=1 where it can, so only a run
        // without the CLI and without that setting skips this check.
        Err(e)
            if e.kind() == std::io::ErrorKind::NotFound
                && !std::env::var("EVALSIG_REQUIRE_AGE").is_ok_and(|v| v == "1") =>
        {
            eprintln!("age CLI not installed; skipped (set EVALSIG_REQUIRE_AGE=1 to require it)");
            return;
        }
        Err(e) => panic!("the age CLI cannot run: {e}"),
    };
    let labels: Vec<Value> = String::from_utf8(plain)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        labels.len() as u64,
        v["results"]["labels"].as_u64().unwrap()
    );
    assert!(
        labels
            .iter()
            .all(|l| l["record"] == "label" && s.held.contains(l["item_id"].as_str().unwrap()))
    );
    // The sealed file is not plaintext.
    let bytes = std::fs::read(file).unwrap();
    assert!(bytes.starts_with(b"age-encryption.org/v1\n"));
    assert!(!String::from_utf8_lossy(&bytes).contains("\"label\""));
}

#[test]
fn sealing_refuses_when_the_agent_holds_holdout_labels() {
    let s = setup();
    let agent = project(&std::fs::read_to_string(SYNTHETIC).unwrap());
    let out = seal(&s, agent.path());
    assert_eq!(out.status.code(), Some(3));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "holdout_labels_in_agent");
    assert!(
        !s.home
            .path()
            .join("sealed")
            .read_dir()
            .unwrap()
            .any(|_| true)
    );
}

#[test]
fn labels_in_a_replaced_snapshot_still_stop_the_seal() {
    let s = setup();
    let agent = project(&std::fs::read_to_string(SYNTHETIC).unwrap());
    // Excluding the holdout items replaces the snapshot; the old one stays stored.
    evalsig(agent.path())
        .arg("exclude")
        .args(&s.held)
        .args(["--reason", "duplicate"])
        .assert()
        .success();
    let out = seal(&s, agent.path());
    assert_eq!(out.status.code(), Some(3));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "holdout_labels_in_agent");
}

#[test]
fn a_line_this_version_cannot_read_hides_no_label() {
    let s = setup();
    let agent = project(&agent_records(&s.held));
    let item = s.held.iter().next().unwrap();
    // A stored object with an unreadable line before a holdout label.
    let object = format!(
        "{{\"record\":\"from_the_future\"}}\n{{\"record\":\"label\",\"item_id\":\"{item}\",\"rater_id\":\"rater:a\",\"source\":\"human\",\"value\":true,\"version\":1}}"
    );
    evalsig_store::objects::ObjectStore::new(agent.path().join(".evalsig/objects"))
        .put(object.as_bytes())
        .unwrap();
    let out = seal(&s, agent.path());
    assert_eq!(out.status.code(), Some(3));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "holdout_labels_in_agent");
}

#[test]
fn a_revision_is_resealed_and_logged_by_hash_only() {
    let s = setup();
    let agent = project(&agent_records(&s.held));
    assert!(seal(&s, agent.path()).status.success());
    // The referee revises one holdout label.
    let item = s.held.iter().next().unwrap();
    let revision = s.referee_project.path().join("revision.jsonl");
    std::fs::write(
        &revision,
        format!(r#"{{"record":"label","item_id":"{item}","rater_id":"lead","source":"human","value":true,"version":2,"kind":"revision"}}"#) + "\n",
    )
    .unwrap();
    evalsig(s.referee_project.path())
        .arg("import")
        .arg(&revision)
        .assert()
        .success();
    let out = seal(&s, agent.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["results"]["resealed"], true);
    let ledger = std::fs::read_to_string(s.home.path().join("ledger.jsonl")).unwrap();
    let revisions: Vec<Value> = ledger
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|e| e["kind"] == "label_revision")
        .collect();
    assert_eq!(revisions.len(), 1);
    let summary = revisions[0]["summary"].to_string();
    assert!(
        summary.contains("blake3:") && !summary.contains(item) && !summary.contains("\"value\""),
        "{summary}"
    );
}
