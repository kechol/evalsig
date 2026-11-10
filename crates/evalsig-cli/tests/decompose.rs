//! `report --decompose` (D-048): each error goes to its first cause in `[attribution]
//! order`, with shares and ceiling rates checked against a hand count.

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .env_remove("AI_AGENT")
        .arg("--project")
        .arg(dir);
    c
}

/// 40 positives, the first 20 missed; 40 negatives, the first 8 false alarms. The
/// misses carry: 10 `no_rule, fixable`, 6 `fixable`, 2 `needs_external_info, no_rule`
/// and 2 nothing.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    evalsig(dir.path()).arg("init").assert().success();
    std::fs::write(
        dir.path().join(".evalsig/config.toml"),
        "[project]\nname = \"d\"\npositive = { equals = true }\ndecision = { equals = true }\n[attribution]\norder = [\"needs_external_info\", \"no_rule\", \"fixable\"]\n",
    )
    .unwrap();
    let mut l = vec![
        r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#
            .to_owned(),
    ];
    for i in 0..80 {
        let positive = i < 40;
        let judged = if positive { i >= 20 } else { i < 48 };
        l.push(format!(r#"{{"record":"item","item_id":"i{i:02}"}}"#));
        l.push(format!(
            r#"{{"record":"label","item_id":"i{i:02}","rater_id":"ana","source":"human","value":{positive},"version":1}}"#
        ));
        l.push(format!(
            r#"{{"record":"judgment","item_id":"i{i:02}","run_id":"r","gen_replicate":0,"output_type":"binary","value":{judged}}}"#
        ));
        let causes = match i {
            0..10 => Some(r#""no_rule","fixable""#),
            10..16 => Some(r#""fixable""#),
            16..18 => Some(r#""needs_external_info","no_rule""#),
            _ => None,
        };
        if let Some(c) = causes {
            l.push(format!(
                r#"{{"record":"attribution","item_id":"i{i:02}","run_id":"r","rater_id":"agent:review","causes":[{c}]}}"#
            ));
        }
    }
    let file = dir.path().join("r.jsonl");
    std::fs::write(&file, l.join("\n") + "\n").unwrap();
    evalsig(dir.path())
        .arg("import")
        .arg(&file)
        .assert()
        .success();
    dir
}

#[test]
fn errors_go_to_their_first_cause_in_order() {
    let dir = project();
    let out = evalsig(dir.path())
        .args(["--format", "json", "report", "r", "--decompose", "misses"])
        .args([
            "--ceiling",
            "fixable",
            "--ceiling",
            "needs_external_info,no_rule,fixable",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let d = &v["results"]["decomposition"];
    assert_eq!(d["errors"], "misses");
    assert_eq!(d["rate"], 0.5);
    assert_eq!(d["n_errors"], 20.0);
    let shares: Vec<(&str, f64)> = d["causes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["cause"].as_str().unwrap(), c["share"].as_f64().unwrap()))
        .collect();
    assert_eq!(
        shares,
        [
            ("needs_external_info", 0.1),
            ("no_rule", 0.5),
            ("fixable", 0.3),
            ("unattributed", 0.1)
        ]
    );
    let rates: Vec<f64> = d["ceilings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["rate"].as_f64().unwrap())
        .collect();
    assert_eq!(rates, [0.35, 0.05]);
    assert!(d["ceilings"][0]["ci"].is_array(), "{d}");
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "ceiling_is_optimistic"),
        "{v}"
    );
    // False alarms carry no attributions here: all unattributed.
    let out = evalsig(dir.path())
        .args([
            "--format",
            "json",
            "report",
            "r",
            "--decompose",
            "false_alarms",
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let d = &v["results"]["decomposition"];
    assert_eq!(d["rate"], 0.2);
    assert_eq!(d["causes"][3]["cause"], "unattributed");
    assert_eq!(d["causes"][3]["share"], 1.0);
    // Markdown shows the table.
    let out = evalsig(dir.path())
        .args([
            "--format",
            "md",
            "report",
            "r",
            "--decompose",
            "misses",
            "--ceiling",
            "fixable",
        ])
        .output()
        .unwrap();
    let md = String::from_utf8(out.stdout).unwrap();
    assert!(
        md.contains("| no_rule |") && md.contains("Without fixable"),
        "{md}"
    );
    // A ceiling naming no cause of these errors is refused, with the causes named.
    let out = evalsig(dir.path())
        .args([
            "--format",
            "json",
            "report",
            "r",
            "--decompose",
            "misses",
            "--ceiling",
            "fixabel",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    let e: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "unknown_name");
}
