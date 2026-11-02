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
