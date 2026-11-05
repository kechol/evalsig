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

/// The release workflow refuses a tag without a CHANGELOG section (T-114).
#[cfg(unix)]
#[test]
fn a_tag_needs_a_changelog_section() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/check_changelog.sh"
    );
    let run = |tag: &str| {
        std::process::Command::new("sh")
            .arg(script)
            .arg(tag)
            .output()
            .unwrap()
            .status
            .code()
    };
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        run(&version),
        Some(0),
        "CHANGELOG.md needs a section for {version}"
    );
    assert_eq!(run("v999.0.0"), Some(1));
    let workflow = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.github/workflows/release.yml"
    ))
    .unwrap();
    assert!(workflow.contains("scripts/check_changelog.sh \"$GITHUB_REF_NAME\""));
}
