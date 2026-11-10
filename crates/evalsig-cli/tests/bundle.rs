//! Bundles (D-050): a project's records exported with a manifest reproduce its reports
//! elsewhere; hashed IDs leave no item ID, free text or key behind.

mod common;

use common::{CONFIG, SYNTHETIC, evalsig, evalsig_json};
use serde_json::Value;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    dir
}

#[test]
fn a_bundle_reproduces_the_report_elsewhere() {
    let (a, b, out) = (project(), project(), tempfile::tempdir().unwrap());
    evalsig(a.path())
        .args(["import", SYNTHETIC])
        .assert()
        .success();
    let report = evalsig_json(a.path())
        .args(["report", "triage-v1"])
        .output()
        .unwrap();
    let saved = out.path().join("report.json");
    std::fs::write(&saved, &report.stdout).unwrap();
    let bundle = out.path().join("bundle");
    evalsig(a.path())
        .args(["export", "--to", "bundle", "--out"])
        .arg(&bundle)
        .assert()
        .success();
    evalsig(b.path())
        .arg("import")
        .arg("--from-bundle")
        .arg(&bundle)
        .assert()
        .success();
    let v: Value = serde_json::from_slice(
        &evalsig_json(b.path())
            .arg("verify")
            .arg(&saved)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(v["results"]["match"], true, "{v}");
    // A changed records file is refused.
    let records = bundle.join("records.jsonl");
    let text = std::fs::read_to_string(&records).unwrap();
    std::fs::write(&records, text.replacen("triage-v1", "triage-vX", 1)).unwrap();
    let c = project();
    let out = evalsig_json(c.path())
        .arg("import")
        .arg("--from-bundle")
        .arg(&bundle)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "bundle_mismatch");
}

#[test]
fn hashed_ids_leave_no_id_text_or_key_and_still_report() {
    let (a, b, out) = (project(), project(), tempfile::tempdir().unwrap());
    evalsig(a.path())
        .args(["import", SYNTHETIC])
        .assert()
        .success();
    let bundle = out.path().join("bundle");
    let key = "a-secret-key-for-the-test";
    evalsig(a.path())
        .env("EVALSIG_BUNDLE_KEY", key)
        .args(["export", "--to", "bundle", "--hash-ids", "--out"])
        .arg(&bundle)
        .assert()
        .success();
    let text = std::fs::read_to_string(bundle.join("records.jsonl")).unwrap()
        + &std::fs::read_to_string(bundle.join("manifest.json")).unwrap();
    for leak in ["t-0000", "thread-000", "\"chat\"", key] {
        assert!(!text.contains(leak), "{leak}");
    }
    let out = evalsig_json(b.path())
        .arg("import")
        .arg("--from-bundle")
        .arg(&bundle)
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "bundle_hashed_ids"),
        "{v}"
    );
    let report = |p: &std::path::Path| -> Vec<(Value, Value)> {
        let v: Value = serde_json::from_slice(
            &evalsig_json(p)
                .args(["report", "triage-v1"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        // Hashed clusters sort differently, so the resamples and intervals differ.
        v["results"]["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["metric"].clone(), m["value"].clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(report(a.path()), report(b.path()));
    // A short key, or hashing outside a bundle, is refused.
    for args in [
        &["export", "--to", "bundle", "--hash-ids", "--out"][..],
        &["export", "--to", "jsonl", "--hash-ids", "--out"],
    ] {
        let out = evalsig_json(a.path())
            .env("EVALSIG_BUNDLE_KEY", "short")
            .args(args)
            .arg(bundle.join("refused"))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64), "{args:?}");
    }
    // Without the key, hashing is refused.
    let out = evalsig_json(a.path())
        .env_remove("EVALSIG_BUNDLE_KEY")
        .args(["export", "--to", "bundle", "--hash-ids", "--out"])
        .arg(bundle.join("again"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
}
