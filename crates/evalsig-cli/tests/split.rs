//! `evalsig split`, `evalsig splits` and `compare --split` (SPEC §8.1, D-022).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use assert_cmd::Command;
use serde_json::{Value, json};

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/synthetic/records.jsonl"
);
const CONFIG: &str = include_str!("../../../examples/synthetic/config.toml");

fn evalsig(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("evalsig").unwrap();
    c.env("EVALSIG_ACTOR", "user:test")
        .arg("--project")
        .arg(dir);
    c
}

fn json_of(c: &mut Command) -> Value {
    serde_json::from_slice(
        &c.args(["--format", "json"])
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .unwrap()
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

/// Item ID → cluster, from `evalsig items`.
fn clusters(dir: &Path) -> BTreeMap<String, String> {
    json_of(evalsig(dir).arg("items"))["results"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["item_id"].as_str().unwrap().to_owned(),
                i["cluster"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn ids(v: &Value) -> BTreeSet<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn splits_keep_clusters_whole_and_are_listed_in_every_format() {
    let dir = project();
    let v = json_of(evalsig(dir.path()).args(["split", "--stratum", "channel"]));
    // The synthetic items carry no occurred_at, so the default is random.
    assert_eq!(v["results"]["method"], "random");
    assert_eq!(v["results"]["holdout_excluded"], false);
    let file: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(v["results"]["file"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let (dev, val) = (ids(&file["dev"]), ids(&file["validation"]));
    assert_eq!(dev.len() + val.len(), 300);
    assert_eq!(v["results"]["validation"]["items"], val.len());
    let cl = clusters(dir.path());
    let dev_clusters: BTreeSet<&String> = dev.iter().map(|i| &cl[i]).collect();
    assert!(
        val.iter().all(|i| !dev_clusters.contains(&cl[i])),
        "a cluster crosses the split"
    );

    let ledger = std::fs::read_to_string(dir.path().join(".evalsig/ledger.jsonl")).unwrap();
    let last: Value = serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "split");
    assert_eq!(last["summary"]["validation"], v["results"]["validation"]);
    assert_eq!(last["summary"]["seed"], 20261103);

    let listed = json_of(evalsig(dir.path()).arg("splits"));
    assert_eq!(listed["results"]["count"], 1);
    assert_eq!(
        listed["results"]["splits"][0]["validation"],
        v["results"]["validation"]
    );
    for format in ["text", "md"] {
        let out = evalsig(dir.path())
            .args(["--format", format, "splits"])
            .assert()
            .success();
        let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
        assert!(
            text.contains(v["results"]["validation"]["hash"].as_str().unwrap()),
            "{format}: {text}"
        );
    }
}

#[test]
fn requests_with_sealed_items_or_split_clusters_are_refused() {
    let dir = project();
    let cl = clusters(dir.path());
    // The judge withheld thread-059 for the holdout.
    let remaining: Vec<&String> = cl
        .iter()
        .filter(|(_, c)| *c != "thread-059")
        .map(|(i, _)| i)
        .collect();
    let withheld: Vec<&String> = cl
        .iter()
        .filter(|(_, c)| *c == "thread-059")
        .map(|(i, _)| i)
        .collect();
    let within = dir.path().join("remaining.json");
    std::fs::write(
        &within,
        json!({"schema": "evalsig.remaining/1", "holdout": "blake3:00", "items": remaining})
            .to_string(),
    )
    .unwrap();
    let v = json_of(
        evalsig(dir.path())
            .arg("split")
            .arg("--within")
            .arg(&within),
    );
    assert_eq!(v["results"]["holdout"], "blake3:00");
    assert_eq!(
        v["results"]["dev"]["items"].as_u64().unwrap()
            + v["results"]["validation"]["items"].as_u64().unwrap(),
        remaining.len() as u64
    );

    let error = |args: &[&str], assign: &Value| -> Value {
        let path = dir.path().join("assign.json");
        std::fs::write(&path, assign.to_string()).unwrap();
        let out = evalsig(dir.path())
            .args(["--format", "json", "split", "--assign"])
            .arg(&path)
            .args(args)
            .assert()
            .failure();
        serde_json::from_slice(&out.get_output().stderr).unwrap()
    };
    // Whole clusters, but one of them is sealed.
    let mut assign: BTreeMap<&String, &str> = cl.keys().map(|i| (i, "dev")).collect();
    for i in &withheld {
        assign.insert(i, "validation");
    }
    let e = error(&["--within", within.to_str().unwrap()], &json!(assign));
    assert_eq!(e["error"]["code"], "holdout_item");
    // One cluster on both sides.
    let first = cl.iter().find(|(_, c)| *c == "thread-000").unwrap().0;
    let mut crossing: BTreeMap<&String, &str> = cl.keys().map(|i| (i, "dev")).collect();
    crossing.insert(first, "validation");
    assert_eq!(
        error(&[], &json!(crossing))["error"]["code"],
        "split_crosses_cluster"
    );
}

#[test]
fn compare_on_the_validation_part_counts_uses_by_its_hash() {
    let dir = project();
    let s = json_of(evalsig(dir.path()).args(["split", "--validation", "0.4"]));
    let c = json_of(evalsig(dir.path()).args([
        "compare",
        "triage-v1",
        "triage-v2",
        "--split",
        "validation",
    ]));
    let r = &c["results"];
    assert_eq!(r["split"], s["results"]["validation"]["hash"]);
    assert_eq!(r["split_part"], "validation");
    assert_eq!(r["n_used"], s["results"]["validation"]["items"]);
    assert_eq!(r["validation_uses"], 1);
    let again = json_of(evalsig(dir.path()).args([
        "compare",
        "triage-v1",
        "triage-v2",
        "--split",
        "validation",
    ]));
    assert_eq!(again["results"]["validation_uses"], 2);

    let saved = dir.path().join(".evalsig/reports").join(format!(
        "{}.json",
        c["provenance"]["output"]
            .as_str()
            .unwrap()
            .strip_prefix("blake3:")
            .unwrap()
    ));
    // A newer split does not change what the saved comparison used.
    evalsig(dir.path())
        .args(["split", "--seed", "9"])
        .assert()
        .success();
    let v = json_of(evalsig(dir.path()).arg("verify").arg(&saved));
    assert_eq!(
        v["results"]["match"], true,
        "{}",
        v["results"]["mismatches"]
    );
}
