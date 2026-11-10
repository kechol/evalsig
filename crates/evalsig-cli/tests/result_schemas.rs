//! The published result schemas (`evalsig schema report|compare`) hold for every mode of
//! the commands; `common::check_results` applies them wherever tests parse a report or a
//! comparison, and this file covers the modes no other test reaches.

mod common;

use assert_cmd::Command;
use serde_json::Value;

use common::{CONFIG, SYNTHETIC, check_results, evalsig, evalsig_json};

#[test]
fn report_and_compare_results_follow_their_schemas() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    evalsig(d).arg("init").assert().success();
    // Expected cost per stratum too.
    let config = CONFIG.to_owned()
        + "\n[cost]\nmatrix = [[0, 1], [5, 0]]\nprevalence = 0.1\nby = \"channel\"\n[cost.strata.chat]\nmix = 0.7\n[cost.strata.email]\nmix = 0.3\n";
    std::fs::write(d.join(".evalsig/config.toml"), config).unwrap();
    evalsig(d).args(["import", SYNTHETIC]).assert().success();
    for args in [
        &["report", "triage-v2", "--features", "--items"][..],
        &["report", "triage-v1", "--by", "channel"],
        &["report", "triage-v1", "--k-curve"],
        &[
            "report",
            "triage-v1",
            "--decompose",
            "false_alarms",
            "--ceiling",
            "unattributed",
        ],
        &[
            "compare",
            "triage-v1",
            "triage-v2",
            "--scratch",
            "--sdt",
            "--features",
            "--items",
        ],
        &[
            "compare",
            "triage-v1",
            "triage-v2",
            "--scratch",
            "--sequential",
        ],
        &[
            "compare",
            "triage-v1",
            "triage-v2",
            "--scratch",
            "--unpaired",
        ],
        &[
            "compare",
            "triage-v1",
            "triage-v2",
            "triage-v1",
            "--scratch",
        ],
    ] {
        let out = evalsig_json(d).args(args).output().unwrap();
        assert!(
            out.status.code().is_some_and(|c| c < 64),
            "{args:?}: {out:?}"
        );
        check_results(&serde_json::from_slice(&out.stdout).unwrap());
    }
}

#[test]
fn the_cli_publishes_the_schemas_the_tests_apply() {
    for (name, file) in [
        (
            "report",
            include_str!("../../evalsig-project/schema/report.schema.json"),
        ),
        (
            "compare",
            include_str!("../../evalsig-project/schema/compare.schema.json"),
        ),
    ] {
        let out = Command::cargo_bin("evalsig")
            .unwrap()
            .args(["schema", name, "--raw"])
            .output()
            .unwrap();
        let published: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            published,
            serde_json::from_str::<Value>(file).unwrap(),
            "{name}"
        );
    }
}
