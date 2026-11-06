use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn version_prints_package_version() {
    Command::cargo_bin("evalsig")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(concat!("evalsig ", env!("CARGO_PKG_VERSION"))));
}

/// The release workflow refuses a tag without a CHANGELOG section.
#[cfg(unix)]
#[test]
fn a_tag_needs_a_changelog_section() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/check_changelog.sh"
    );
    let dir = tempfile::tempdir().unwrap();
    let changelog = dir.path().join("CHANGELOG.md");
    std::fs::write(
        &changelog,
        "# Changelog\n\n## [Unreleased]\n\n## [1.1.0] - Unreleased\n\n## [1.0.0+build.7] - 2026-11-01\n\n## [1.0.0] - 2026-10-01\n",
    )
    .unwrap();
    let run = |tag: &str| {
        std::process::Command::new("sh")
            .arg(script)
            .arg(tag)
            .arg(&changelog)
            .output()
            .unwrap()
            .status
            .code()
    };
    assert_eq!(run("v1.0.0"), Some(0));
    assert_eq!(run("releases/v1.0.0"), Some(0));
    assert_eq!(run("1.0.0+build.7"), Some(0));
    // Still marked Unreleased, or not there at all.
    assert_eq!(run("v1.1.0"), Some(1));
    assert_eq!(run("v999.0.0"), Some(1));
    // The repository's own changelog has a section for this version.
    let own = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md"))
        .unwrap();
    let heading = format!("## [{}]", env!("CARGO_PKG_VERSION"));
    assert!(
        own.lines().any(|l| l.starts_with(&heading)),
        "CHANGELOG.md needs a section for {}",
        env!("CARGO_PKG_VERSION")
    );
    let workflow = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.github/workflows/release.yml"
    ))
    .unwrap();
    assert!(workflow.contains("scripts/check_changelog.sh \"$GITHUB_REF_NAME\""));
}
