use assert_cmd::Command;
use serde_json::Value;

fn evalsig() -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env_remove("AI_AGENT");
    c
}

fn normalize(text: &str) -> Value {
    let v = env!("CARGO_PKG_VERSION");
    let text = text
        .replace(&format!("evalsig {v}"), "evalsig <VERSION>")
        .replace(
            &format!("\"version\": \"{v}\""),
            "\"version\": \"<VERSION>\"",
        );
    serde_json::from_str(&text).unwrap()
}

#[test]
fn envelope_shape_is_fixed() {
    let out = evalsig()
        .args(["version", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let actual = normalize(std::str::from_utf8(&out).unwrap());
    let expected = normalize(include_str!("snapshots/version.json"));
    assert_eq!(actual, expected);
    let keys: Vec<&str> = actual
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "next",
            "ok",
            "provenance",
            "results",
            "schema",
            "truncated",
            "verdict",
            "warnings"
        ]
    );
}

#[test]
fn exit_and_verdict_codes_are_published() {
    let out = evalsig()
        .args(["version", "--format=json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let codes: Vec<u64> = v["results"]["exit_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["code"].as_u64().unwrap())
        .collect();
    assert_eq!(codes, [0, 1, 2, 3, 4, 5, 64]);
    assert_eq!(v["results"]["verdict_codes"].as_array().unwrap().len(), 14);
}

#[test]
fn usage_errors_exit_64() {
    evalsig().arg("no-such-command").assert().code(64);
    evalsig()
        .args(["version", "--no-such-flag"])
        .assert()
        .code(64);
    evalsig()
        .args(["version", "--format", "yaml"])
        .assert()
        .code(64);
    evalsig().assert().code(64);
}

#[test]
fn usage_errors_are_json_when_json_is_requested() {
    let out = evalsig()
        .args(["--format", "json", "no-such-command"])
        .assert()
        .code(64)
        .get_output()
        .stderr
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["error"]["code"], "unknown_command");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no-such-command")
    );
    assert_eq!(v["error"]["next"][0]["command"], "evalsig help --json");
}

#[test]
fn every_format_renders() {
    for (format, needle) in [
        ("text", "records: records/1"),
        ("md", "### `evalsig.version/1`"),
        (
            "junit",
            "<testsuite name=\"evalsig.version/1\" tests=\"1\" failures=\"0\">",
        ),
    ] {
        let out = evalsig()
            .args(["version", "--format", format])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(needle), "{format}: {s}");
    }
}

#[test]
fn help_and_version_flags_exit_0() {
    evalsig().arg("--help").assert().success();
    evalsig().arg("--version").assert().success();
}
