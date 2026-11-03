//! Golden files for the Markdown of `report` and `compare`.

use std::path::Path;

use assert_cmd::Command;

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .args(["--format", "md", "--project"])
        .arg(dir);
    c
}

fn md(c: &mut Command) -> String {
    let out = String::from_utf8(c.assert().success().get_output().stdout.clone()).unwrap();
    out.replace(
        concat!("evalsig ", env!("CARGO_PKG_VERSION")),
        "evalsig <VERSION>",
    )
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    evalsig(dir.path())
        .args(["import", SYNTHETIC])
        .assert()
        .success();
    dir
}

#[test]
fn report_markdown_matches_its_golden_file() {
    let dir = project();
    assert_eq!(
        md(evalsig(dir.path()).args(["report", "triage-v1"])),
        include_str!("snapshots/report.md")
    );
}

#[test]
fn compare_markdown_matches_its_golden_file() {
    let dir = project();
    assert_eq!(
        md(evalsig(dir.path()).args(["compare", "triage-v1", "triage-v2", "--scratch"])),
        include_str!("snapshots/compare.md")
    );
}
