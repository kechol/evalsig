//! `evalsig trend` on a dated fixture (SPEC §10.1, metrics spec §7.3).

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .arg("--project")
        .arg(dir);
    c
}

/// Four ISO weeks (2026-W41 … W44) of 60 items each. The run is right on 9 in 10
/// items, except in the last week, where it is right on 6 in 10. Two more items have
/// no occurred_at.
fn project() -> tempfile::TempDir {
    let mut l = vec![
        r#"{"record":"run","run_id":"prod","system":{"id":"s","version":"1"},"output_type":"binary"}"#.to_owned(),
    ];
    for week in 0..4 {
        for k in 0..60 {
            let id = format!("w{week}-{k:02}");
            // Monday 2026-10-05 is in 2026-W41.
            let day = 5 + week * 7 + k % 5;
            let at = format!("2026-10-{day:02}T09:00:00Z");
            let truth = k % 2 == 0;
            let wrong = if week == 3 { k % 10 < 4 } else { k % 10 == 0 };
            l.push(format!(
                r#"{{"record":"item","item_id":"{id}","occurred_at":"{at}"}}"#
            ));
            l.push(format!(
                r#"{{"record":"judgment","item_id":"{id}","run_id":"prod","gen_replicate":0,"output_type":"binary","value":{}}}"#,
                truth != wrong
            ));
            l.push(format!(
                r#"{{"record":"label","item_id":"{id}","rater_id":"ana","source":"human","value":{truth},"version":1}}"#
            ));
        }
    }
    for id in ["u1", "u2"] {
        l.push(format!(r#"{{"record":"item","item_id":"{id}"}}"#));
        l.push(format!(
            r#"{{"record":"judgment","item_id":"{id}","run_id":"prod","gen_replicate":0,"output_type":"binary","value":true}}"#
        ));
    }
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    let file = dir.path().join("records.jsonl");
    std::fs::write(&file, l.join("\n") + "\n").unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    dir
}

fn trend(dir: &Path, args: &[&str]) -> Value {
    let out = evalsig(dir)
        .args(["--format", "json", "trend", "prod"])
        .args(args)
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

fn status(v: &Value, period: &str) -> String {
    let p = v["results"]["periods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["period"] == period)
        .unwrap();
    p["vs_baseline"][0]["status"]
        .as_str()
        .unwrap_or("baseline")
        .to_owned()
}

#[test]
fn the_last_week_drifts_from_the_first() {
    let dir = project();
    let v = trend(dir.path(), &["--by", "week"]);
    let r = &v["results"];
    let periods: Vec<&str> = r["periods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["period"].as_str().unwrap())
        .collect();
    assert_eq!(periods, ["2026-W41", "2026-W42", "2026-W43", "2026-W44"]);
    assert_eq!(r["baseline_period"], "2026-W41");
    assert_eq!(r["periods"][0]["n_items"], 60);
    assert_eq!(status(&v, "2026-W42"), "stable");
    assert_eq!(status(&v, "2026-W44"), "drifted");
    assert_eq!(v["verdict"]["code"], "drifted");
    assert_eq!(r["undated_items"], 2);
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "undated_items")
    );
}

#[test]
fn since_and_the_baseline_period_choose_the_comparison() {
    let dir = project();
    let v = trend(
        dir.path(),
        &[
            "--since",
            "2026-10-12T00:00:00Z",
            "--baseline-period",
            "2026-W43",
        ],
    );
    let periods: Vec<&str> = v["results"]["periods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["period"].as_str().unwrap())
        .collect();
    assert_eq!(periods, ["2026-W42", "2026-W43", "2026-W44"]);
    assert_eq!(status(&v, "2026-W42"), "stable");
    assert_eq!(status(&v, "2026-W43"), "baseline");
    assert_eq!(v["verdict"]["code"], "drifted");

    let month = trend(dir.path(), &["--by", "month"]);
    assert_eq!(month["results"]["periods"][0]["period"], "2026-10");
    assert!(month["verdict"].is_null());

    evalsig(dir.path())
        .args(["trend", "prod", "--baseline-period", "2026-W01"])
        .assert()
        .failure()
        .code(64);
}
