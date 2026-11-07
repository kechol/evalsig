//! `evalsig preregister --draft` (SPEC §8.3): the ported example drafts with a stable
//! hash that does not depend on comments, layout or key order.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/gate.toml");
/// The hash of the example's canonical form; a change means the example or the
/// canonical form changed.
const EXAMPLE_HASH: &str =
    "blake3:55b72281fb757c80b0faacaf133d2b22bd254de37be4c06f713752353a830de7";

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .arg("--project")
        .arg(dir);
    c
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    dir
}

fn draft(dir: &Path, file: &Path) -> std::process::Output {
    evalsig(dir)
        .args(["--format", "json", "preregister", "--draft"])
        .arg(file)
        .output()
        .unwrap()
}

#[test]
fn the_example_drafts_with_a_stable_hash() {
    let dir = project();
    let out = draft(dir.path(), Path::new(EXAMPLE));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["results"]["draft"], EXAMPLE_HASH);
    assert_eq!(v["results"]["approved"], false);
    let ledger = std::fs::read_to_string(dir.path().join(".evalsig/ledger.jsonl")).unwrap();
    let line: Value = serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
    assert_eq!(line["kind"], "preregister_draft");
    assert_eq!(line["outputs"][0], EXAMPLE_HASH);

    // Without comments and with every key in reverse order: same hash.
    let text = std::fs::read_to_string(EXAMPLE).unwrap();
    let doc: toml::Table = toml::from_str(&text).unwrap();
    let is_table = |v: &toml::Value| {
        v.is_table()
            || v.as_array()
                .is_some_and(|a| a.iter().all(toml::Value::is_table))
    };
    let (tables, plain): (Vec<_>, Vec<_>) = doc.iter().rev().partition(|(_, v)| is_table(v));
    let mut reordered = String::new();
    // Top-level keys first: after a [table] header they would belong to it.
    for (k, v) in plain.into_iter().chain(tables) {
        let one = toml::Table::from_iter([(k.clone(), v.clone())]);
        reordered.push_str(&toml::to_string(&one).unwrap());
    }
    let copy = dir.path().join("reordered.toml");
    std::fs::write(&copy, reordered).unwrap();
    let again: Value = serde_json::from_slice(&draft(dir.path(), &copy).stdout).unwrap();
    assert_eq!(again["results"]["draft"], EXAMPLE_HASH);
}

#[test]
fn drafts_outside_the_schema_are_refused() {
    let dir = project();
    let text = std::fs::read_to_string(EXAMPLE).unwrap();
    for (what, changed) in [
        ("no sidedness", text.replacen("sided = \"one\"\n", "", 1)),
        ("unknown key", format!("colour = \"blue\"\n{text}")),
        (
            "resolved by hand",
            format!("{text}\n[resolved]\nlevels = 4\n"),
        ),
        (
            "pass on another metric",
            text.replacen("rule = \"not_worse_than\"", "rule = \"pass\"", 1),
        ),
    ] {
        let file = dir.path().join("bad.toml");
        std::fs::write(&file, changed).unwrap();
        let out = draft(dir.path(), &file);
        assert_eq!(
            out.status.code(),
            Some(3),
            "{what}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        let e: Value = serde_json::from_slice(&out.stderr).unwrap();
        assert_eq!(e["error"]["code"], "invalid_conditions", "{what}");
    }
    // The spec's spelling of a boolean positive definition is accepted.
    let file = dir.path().join("boolean.toml");
    std::fs::write(
        &file,
        text.replace(
            "positive = { ordinal_at_least = 2 }",
            "positive = { boolean = true }",
        ),
    )
    .unwrap();
    assert!(draft(dir.path(), &file).status.success());
}
