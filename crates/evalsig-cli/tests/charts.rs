//! The dashboard's charts (SPEC §11 "Rendering"): drawn from the CLI's own JSON, so
//! every value they show is the value the CLI printed.

use std::path::Path;

use evalsig_dashboard::svg;
use serde_json::Value;

mod common;
use common::evalsig_json;

const RECORDS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/records.jsonl");
const CONFIG: &str = "[project]\nname = \"example\"\npositive = { ordinal_at_least = 2 }\ndecision = { ordinal_at_least = 2 }\n[stats]\nseed = 20261029\n";

fn out(dir: &Path, args: &[&str]) -> Value {
    let o = evalsig_json(dir).args(args).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

fn rows<'a>(metrics: &'a Value, point: &str) -> Vec<svg::Row<'a>> {
    metrics
        .as_array()
        .unwrap()
        .iter()
        .map(|m| svg::Row {
            label: m["metric"].as_str().unwrap().to_owned(),
            point: &m[point],
            ci: &m["ci"],
        })
        .collect()
}

#[test]
fn every_drawn_value_is_the_cli_value() {
    let dir = tempfile::tempdir().unwrap();
    evalsig_json(dir.path()).arg("init").assert().success();
    std::fs::write(dir.path().join(".evalsig/config.toml"), CONFIG).unwrap();
    evalsig_json(dir.path())
        .args(["import", RECORDS])
        .assert()
        .success();
    let runs = out(dir.path(), &["runs"]);
    let ids: Vec<&str> = runs["results"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["run_id"].as_str().unwrap())
        .collect();

    // The report's metrics, ROC and z-ROC.
    let report = out(dir.path(), &["report", ids[0]]);
    let r = &report["results"];
    let plot = svg::intervals(&rows(&r["metrics"], "value"), false);
    for m in r["metrics"].as_array().unwrap() {
        if m["ci"].is_null() {
            continue;
        }
        let attrs = format!(
            "data-label=\"{}\" data-value=\"{}\" data-lo=\"{}\" data-hi=\"{}\"",
            m["metric"].as_str().unwrap(),
            m["value"],
            m["ci"][0],
            m["ci"][1]
        );
        assert!(plot.contains(&attrs), "{attrs}");
    }
    let roc = svg::roc(&r["roc"]);
    let points = r["roc"]["points"].as_array().unwrap();
    assert!(!points.is_empty());
    for p in points {
        assert!(
            roc.contains(&format!("data-x=\"{}\" data-y=\"{}\"", p[0], p[1])),
            "{p}"
        );
    }
    if let Some(z) = r["roc"]["zroc"].as_array() {
        let drawn = svg::zroc(&r["roc"]);
        for p in z {
            assert!(drawn.contains(&format!("data-zx=\"{}\" data-zy=\"{}\"", p[0], p[1])));
        }
    }

    // A trend's primary metric per period.
    let trend = out(dir.path(), &["trend", ids[0], "--by", "week"]);
    let t = &trend["results"];
    let primary = t["primary_metric"].as_str().unwrap();
    let plot = svg::trend(&t["periods"], primary);
    let periods = t["periods"].as_array().unwrap();
    assert!(periods.len() > 1);
    for p in periods {
        let m = p["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["metric"] == primary)
            .unwrap();
        assert!(
            plot.contains(&format!(
                "data-label=\"{}\" data-value=\"{}\"",
                p["period"].as_str().unwrap(),
                m["value"]
            )),
            "{p}"
        );
    }

    // A comparison's differences and transition table.
    let compare = out(dir.path(), &["compare", ids[0], ids[1]]);
    let c = &compare["results"];
    let plot = svg::intervals(&rows(&c["metrics"], "delta"), true);
    for m in c["metrics"].as_array().unwrap() {
        if m["ci"].is_null() {
            continue;
        }
        assert!(plot.contains(&format!(
            "data-value=\"{}\" data-lo=\"{}\" data-hi=\"{}\"",
            m["delta"], m["ci"][0], m["ci"][1]
        )));
    }
    if c["transition"].is_object() {
        let table = svg::transition(&c["transition"]);
        for (k, v) in c["transition"].as_object().unwrap() {
            assert!(table.contains(&format!("data-key=\"{k}\">{v}<")), "{k}");
        }
    }
}
