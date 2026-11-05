//! The quick start (T-121): both language versions give the same commands, and those
//! commands run as written against the synthetic example.

use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The lines of every `sh` block, in order.
fn commands(doc: &str) -> Vec<String> {
    let text = std::fs::read_to_string(Path::new(ROOT).join(doc)).unwrap();
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        match (inside, line.trim()) {
            (false, "```sh") => inside = true,
            (true, "```") => inside = false,
            (true, l) => out.push(l.to_owned()),
            _ => {}
        }
    }
    out
}

#[test]
fn both_versions_give_the_same_commands() {
    let en = commands("docs/quickstart.md");
    assert!(en.len() > 5);
    assert_eq!(en, commands("docs/quickstart.ja.md"));
}

#[cfg(unix)]
#[test]
fn the_commands_run_as_written() {
    // Installing, cloning and serving are left out; the clone is the repository's
    // examples copied into place.
    let script: Vec<String> = commands("docs/quickstart.md")
        .into_iter()
        .filter(|l| {
            !["cargo install", "git clone", "evalsig dashboard"]
                .iter()
                .any(|p| l.starts_with(p))
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let example = dir.path().join("evalsig/examples/synthetic");
    std::fs::create_dir_all(&example).unwrap();
    std::fs::copy(
        Path::new(ROOT).join("examples/synthetic/records.jsonl"),
        example.join("records.jsonl"),
    )
    .unwrap();
    let bin = assert_cmd::cargo::cargo_bin("evalsig");
    let path = format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new("sh")
        .args(["-e", "-c", &script.join("\n")])
        .current_dir(dir.path())
        .env("PATH", path)
        .env("EVALSIG_ACTOR", "user:test")
        .env("EVALSIG_AGENT_MODE", "0")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The verdict line the quick start quotes.
    assert!(
        stdout.contains("improved: accuracy: difference +0.0533"),
        "{stdout}"
    );
}
