//! `evalsig gate` (SPEC §8.3, §8.4; metrics spec §11.2): each submission check, the
//! budget, and resubmission of the same candidate.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

mod common;
use common::*;

fn gate_toml(min_items: usize, budget: u32) -> String {
    format!(
        r#"name = "triage-v2 is no worse"
hypothesis = "The new prompt keeps accuracy"
split = "holdout"
min_items = {min_items}

[compare]
baseline = "triage-v1"
candidate = "*"

[grader]
id = "rule-check"
version = "1"

[[conditions]]
metric = "accuracy"
rule = "not_worse_than"
value = -0.3
sided = "one"

[[conditions]]
metric = "missing_rate"
rule = "upper_bound_at_most"
value = 0.5
sided = "one"

[holdout]
budget = {budget}
"#
    )
}

#[test]
fn a_passing_candidate_learns_only_verdicts_and_a_resubmission_costs_nothing() {
    let w = world(&gate_toml(10, 3), None);
    let records = candidate(w.referee.path(), "cand.jsonl", |_| {});
    let out = submit(&w, &records);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v = json(&out);
    assert_eq!(v["verdict"]["code"], "pass");
    assert_eq!(
        v["results"]["budget"],
        serde_json::json!({"total": 3, "used": 1, "remaining": 2})
    );
    // Verdicts per condition, nothing else: no interval, no value, no warning.
    for c in v["results"]["conditions"].as_array().unwrap() {
        let keys: BTreeSet<&str> = c.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, BTreeSet::from(["metric", "rule", "value", "passed"]));
    }
    assert!(
        v["warnings"].as_array().unwrap().is_empty(),
        "{}",
        v["warnings"]
    );
    let ledger = std::fs::read_to_string(w.home.path().join("ledger.jsonl")).unwrap();
    let gate: Value = serde_json::from_str(
        ledger
            .lines()
            .rfind(|l| l.contains("\"kind\":\"gate\""))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(gate["summary"]["seal_void"], false);
    assert!(gate["summary"]["withheld_warnings"].is_array());

    let again = submit(&w, &records);
    assert_eq!(again.status.code(), Some(0));
    let v = json(&again);
    assert_eq!(v["warnings"][0]["code"], "resubmission");
    assert_eq!(accesses(&w), 1);
}

#[test]
fn missing_holdout_items_are_refused_before_any_budget_is_spent() {
    let w = world(&gate_toml(10, 3), None);
    let left_out = w.held.iter().next().unwrap().clone();
    let records = candidate(w.referee.path(), "short.jsonl", |rows| {
        rows.retain(|r| r["item_id"] != left_out.as_str())
    });
    let out = submit(&w, &records);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(json(&out)["error"]["code"], "incomplete_coverage");
    assert_eq!(accesses(&w), 0);
}

#[test]
fn a_different_grader_or_mapping_is_refused_after_spending_budget() {
    let mapping = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(mapping.path(), "schema = \"mapping/1\"\n").unwrap();
    let w = world(&gate_toml(10, 5), Some(mapping.path()));
    let other = candidate(w.referee.path(), "grader.jsonl", |rows| {
        rows[0]["grader"]["version"] = "2".into()
    });
    let out = submit(&w, &other);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(json(&out)["error"]["code"], "does_not_match_approval");
    assert_eq!(accesses(&w), 1);

    let records = candidate(w.referee.path(), "cand.jsonl", |_| {});
    let out = submit(&w, &records);
    assert_eq!(out.status.code(), Some(3), "no mapping hash given");
    let hash = evalsig_store::hash::ContentHash::blake3(&std::fs::read(mapping.path()).unwrap())
        .to_string();
    let out = submit_to(
        &w,
        &w.approved,
        &candidate(w.referee.path(), "cand2.jsonl", |rows| {
            rows[0]["attrs"] = serde_json::json!({"n": 2})
        }),
        &["--mapping-hash", &hash],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn identical_replicates_stop_the_gate() {
    let w = world(&gate_toml(10, 3), None);
    let records = candidate(w.referee.path(), "copies.jsonl", |rows| {
        let mut extra = Vec::new();
        for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
            r["output_ref"] = format!("blake3:{}", "a".repeat(64)).into();
            let mut copy = r.clone();
            copy["gen_replicate"] = 1.into();
            extra.push(copy);
        }
        rows.extend(extra);
    });
    let out = submit(&w, &records);
    assert_eq!(
        out.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json(&out)["verdict"]["code"], "blocked");
    assert_eq!(accesses(&w), 1);
}

#[test]
fn too_few_paired_items_are_insufficient_whatever_the_labels() {
    // The referee's baseline lacks five holdout items, so only the rest are paired.
    let held = world(&gate_toml(10, 1), None).held;
    let dropped: BTreeSet<String> = held.iter().take(5).cloned().collect();
    let rows: Vec<Value> = lines()
        .into_iter()
        .filter(|v| {
            !(v["record"] == "judgment"
                && v["run_id"] == "triage-v1"
                && dropped.contains(v["item_id"].as_str().unwrap()))
        })
        .collect();
    let w = world_from(&rows, &gate_toml(held.len(), 3), None);
    let out = submit(&w, &candidate(w.referee.path(), "cand.jsonl", |_| {}));
    assert_eq!(
        out.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json(&out)["verdict"]["code"], "insufficient_data");
}

#[test]
fn the_budget_runs_out() {
    let w = world(&gate_toml(10, 2), None);
    for k in 0..2 {
        let records = candidate(w.referee.path(), &format!("c{k}.jsonl"), |rows| {
            rows[0]["attrs"] = serde_json::json!({"try": k})
        });
        assert_eq!(submit(&w, &records).status.code(), Some(0));
    }
    let records = candidate(w.referee.path(), "c9.jsonl", |rows| {
        rows[0]["attrs"] = serde_json::json!({"try": 9})
    });
    let out = submit(&w, &records);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(json(&out)["verdict"]["code"], "budget_exhausted");
    assert_eq!(accesses(&w), 2);
}

#[test]
fn model_derived_holdout_references_stop_the_gate() {
    let rows: Vec<Value> = lines()
        .into_iter()
        .map(|mut v| {
            if v["record"] == "label" {
                v["source"] = "model".into();
            }
            v
        })
        .collect();
    let w = world_from(&rows, &gate_toml(10, 3), None);
    let out = submit(&w, &candidate(w.referee.path(), "cand.jsonl", |_| {}));
    assert_eq!(
        out.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn an_approved_hash_that_is_not_a_blake3_reference_names_no_file() {
    let w = world(&gate_toml(10, 3), None);
    let records = variant(w.referee.path(), 1);
    for bad in ["../../ledger", "blake3:../x", "sha256:00"] {
        let out = submit_to(&w, bad, &records, &[]);
        assert_eq!(code(&out), Some(64), "{bad}");
    }
    assert_eq!(accesses(&w), 0);
}

#[test]
fn absolute_criteria_without_a_baseline_are_judged() {
    let gate = r#"name = "triage-v2 is accurate enough"
hypothesis = "triage-v2 is right on most items"
split = "holdout"
min_items = 10

[compare]
candidate = "*"

[grader]
id = "rule-check"
version = "1"

[[conditions]]
metric = "accuracy"
rule = "lower_bound_at_least"
value = 0.3
sided = "one"

[holdout]
budget = 2
"#;
    let w = world(gate, None);
    let out = submit(&w, &variant(w.referee.path(), 1));
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn a_change_on_the_referee_side_after_approval_spends_no_budget() {
    let w = world(&gate_toml(10, 3), None);
    // Sealed again after approval.
    evalsig(w.referee.path())
        .args(["seal", "--holdout", &w.holdout, "--referee"])
        .arg(w.home.path())
        .arg("--agent-project")
        .arg(w.agent.path())
        .assert()
        .success();
    let out = submit(&w, &variant(w.referee.path(), 1));
    assert_eq!(code(&out), Some(3));
    assert_eq!(json(&out)["error"]["code"], "approval_out_of_date");
    // A holdout item excluded after approval.
    let w = world(&gate_toml(10, 3), None);
    let item = w.held.iter().next().unwrap().clone();
    let v = json(
        &evalsig(w.referee.path())
            .args([
                "--format",
                "json",
                "exclude",
                &item,
                "--reason",
                "duplicate",
                "--referee",
            ])
            .arg(w.home.path())
            .output()
            .unwrap(),
    );
    assert_eq!(v["warnings"][0]["code"], "reapproval_needed");
    let out = submit(&w, &variant(w.referee.path(), 2));
    assert_eq!(code(&out), Some(3));
    assert_eq!(json(&out)["error"]["code"], "approval_out_of_date");
    assert_eq!(accesses(&w), 0);
}

#[test]
fn each_run_of_one_records_file_is_its_own_submission() {
    let w = world(&gate_toml(10, 3), None);
    let both = w.referee.path().join("both.jsonl");
    let rows: Vec<Value> = lines()
        .into_iter()
        .filter(|v| v["record"] == "run" || v["record"] == "judgment")
        .collect();
    std::fs::write(&both, jsonl(&rows)).unwrap();
    let first = submit(&w, &both);
    assert_eq!(code(&first), Some(0));
    let other = evalsig(w.referee.path())
        .args([
            "--format",
            "json",
            "gate",
            "--approved",
            &w.approved,
            "--candidate",
            "triage-v1",
            "--records",
        ])
        .arg(&both)
        .arg("--referee")
        .arg(w.home.path())
        .output()
        .unwrap();
    let v = json(&other);
    assert!(v["warnings"].as_array().is_none_or(|w| w.is_empty()), "{v}");
    assert_eq!(v["results"]["candidate"], "triage-v1");
    assert_eq!(accesses(&w), 2);
}

#[test]
fn a_condition_without_a_threshold_is_not_a_draft() {
    let w = world(&gate_toml(10, 3), None);
    let g = w.agent.path().join("no-value.toml");
    std::fs::write(&g, gate_toml(10, 3).replace("value = -0.3\n", "")).unwrap();
    let out = evalsig(w.agent.path())
        .args(["--format", "json", "preregister", "--draft"])
        .arg(&g)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        json(&out)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("needs a numeric value")
    );
}

#[test]
fn parity_drafts_and_uncomputed_metrics_are_checked_up_front() {
    let w = world(&gate_toml(10, 3), None);
    // human_parity drafts without a value; an epsilon outside (0, 1) is refused.
    let draft = |name: &str, condition: &str| {
        let g = w.agent.path().join(name);
        std::fs::write(
            &g,
            gate_toml(10, 3).replace(
                "metric = \"accuracy\"\nrule = \"not_worse_than\"\nvalue = -0.3",
                condition,
            ),
        )
        .unwrap();
        evalsig(w.agent.path())
            .args(["--format", "json", "preregister", "--draft"])
            .arg(&g)
            .output()
            .unwrap()
    };
    let out = draft("parity.toml", "metric = \"human_parity\"\nrule = \"pass\"");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = draft(
        "wide.toml",
        "metric = \"human_parity\"\nrule = \"pass\"\nvalue = 1.5",
    );
    assert!(!out.status.success());
    assert!(
        json(&out)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("strictly between 0 and 1")
    );
    // A metric the gate does not compute for this run is an error, not undecided.
    let w = world(
        &gate_toml(10, 3).replace(
            "metric = \"accuracy\"\nrule = \"not_worse_than\"\nvalue = -0.3",
            "metric = \"dprime\"\nrule = \"lower_bound_at_least\"\nvalue = 0.0",
        ),
        None,
    );
    let out = submit(&w, &variant(w.referee.path(), 1));
    assert_eq!(
        code(&out),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json(&out)["error"]["code"], "metric_not_computed");
}

#[cfg(unix)]
#[test]
fn a_readable_identity_voids_the_seal_and_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let w = world(&gate_toml(10, 3), None);
    // The agent's user could read the key: the referee runs with the agent's privileges.
    std::fs::set_permissions(
        w.home.path().join("identity.txt"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let out = submit(&w, &variant(w.referee.path(), 1));
    assert_eq!(code(&out), Some(0));
    let v = json(&out);
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["code"] == "seal_void"),
        "{v}"
    );
    let gate = referee_ledger(&w)
        .into_iter()
        .rfind(|e| e["kind"] == "gate")
        .unwrap();
    assert_eq!(gate["summary"]["seal_void"], true);
}

#[test]
fn a_referee_home_inside_the_project_voids_the_seal() {
    let w = world(&gate_toml(10, 3), None);
    let inside = w.referee.path().join("referee-home");
    copy_dir(w.home.path(), &inside);
    let out = evalsig(w.referee.path())
        .args([
            "--format",
            "json",
            "gate",
            "--approved",
            &w.approved,
            "--candidate",
            "triage-v2",
            "--records",
        ])
        .arg(variant(w.referee.path(), 2))
        .arg("--referee")
        .arg(&inside)
        .output()
        .unwrap();
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let ledger = std::fs::read_to_string(inside.join("ledger.jsonl")).unwrap();
    let gate: Value = serde_json::from_str(
        ledger
            .lines()
            .rfind(|l| l.contains("\"kind\":\"gate\""))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(gate["summary"]["seal_void"], true);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), &target).unwrap();
        }
    }
}

#[test]
fn another_output_type_is_refused_after_spending_budget() {
    let w = world(&gate_toml(10, 3), None);
    let records = candidate(w.referee.path(), "score.jsonl", |rows| {
        rows[0]["output_type"] = "score".into();
        for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
            r["output_type"] = "score".into();
            if let Some(b) = r["value"].as_bool() {
                r["value"] = if b { 1.0 } else { 0.0 }.into();
            }
        }
    });
    let out = submit(&w, &records);
    assert_eq!(
        code(&out),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let e = &json(&out)["error"];
    assert_eq!(e["code"], "does_not_match_approval");
    assert!(e["message"].as_str().unwrap().contains("output type"));
    assert_eq!(accesses(&w), 1);
}

#[test]
fn the_gate_answer_renders_as_junit_without_values() {
    let w = world(&gate_toml(10, 3), None);
    let wrong = candidate(w.referee.path(), "wrong.jsonl", |rows| {
        for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
            if let Some(b) = r["value"].as_bool() {
                r["value"] = (!b).into();
            }
        }
    });
    let out = evalsig(w.referee.path())
        .args([
            "--format",
            "junit",
            "gate",
            "--approved",
            &w.approved,
            "--candidate",
            "triage-v2",
            "--records",
        ])
        .arg(&wrong)
        .arg("--referee")
        .arg(w.home.path())
        .output()
        .unwrap();
    assert_eq!(
        code(&out),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let xml = String::from_utf8(out.stdout).unwrap();
    assert!(xml.contains("tests=\"2\""), "{xml}");
    assert!(
        xml.contains("name=\"accuracy not_worse_than -0.3\""),
        "{xml}"
    );
    assert!(xml.contains("<failure message=\"the referee returns the verdict only\"/>"));
    assert!(
        !xml.contains("estimate") && !xml.contains("bound "),
        "{xml}"
    );
}

#[test]
fn the_gate_takes_no_override_flags() {
    let w = world(&gate_toml(10, 3), None);
    let records = variant(w.referee.path(), 1);
    for flag in [
        "--allow-identical-replicates",
        "--allow-input-change",
        "--allow-mixed-grader",
        "--allow-unexplained-missing",
    ] {
        let out = submit_to(&w, &w.approved, &records, &[flag]);
        assert_eq!(code(&out), Some(64), "{flag}");
    }
    assert_eq!(accesses(&w), 0);
}

#[test]
fn approval_warns_of_low_power_and_counts_earlier_queries() {
    let w = world(&gate_toml(10, 3), None);
    assert_eq!(code(&submit(&w, &variant(w.referee.path(), 1))), Some(0));
    // A small improvement to detect on a few dozen items: the gate would rarely pass.
    let v = approve_output(
        &w,
        &gate_toml(10, 3).replace(
            "rule = \"not_worse_than\"\nvalue = -0.3",
            "rule = \"delta_lower_bound_at_least\"\nvalue = 0.0\nmin_effect = 0.01",
        ),
        None,
    );
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["code"] == "low_power_at_hypothesis"),
        "{v}"
    );
    assert!(v["results"]["power_at_hypothesis"].as_f64().unwrap() < 0.5);
    assert_eq!(v["results"]["holdout_access_so_far"], 1);
}

/// The synthetic data with three human raters who agree on every item.
fn three_raters() -> (Vec<Value>, BTreeMap<String, bool>) {
    let mut truth = BTreeMap::new();
    let mut rows = Vec::new();
    for r in lines() {
        if r["record"] == "label" {
            let item = r["item_id"].as_str().unwrap().to_owned();
            if let std::collections::btree_map::Entry::Vacant(slot) = truth.entry(item) {
                slot.insert(r["value"].as_bool().unwrap());
                for rater in ["human:a", "human:b", "human:c"] {
                    let mut l = r.clone();
                    l["rater_id"] = rater.into();
                    rows.push(l);
                }
            }
        } else {
            rows.push(r);
        }
    }
    (rows, truth)
}

#[test]
fn a_parity_condition_passes_a_candidate_that_says_what_people_say() {
    let (referee_rows, truth) = three_raters();
    let gate = gate_toml(10, 3).replace(
        "metric = \"accuracy\"\nrule = \"not_worse_than\"\nvalue = -0.3",
        "metric = \"human_parity\"\nrule = \"pass\"",
    );
    let w = world_from(&referee_rows, &gate, None);
    // The approval fixes epsilon, since the gate reads no configuration.
    let approved = std::fs::read_to_string(
        w.home
            .path()
            .join("approved")
            .join(format!("{}.toml", w.approved.trim_start_matches("blake3:"))),
    )
    .unwrap();
    assert!(approved.contains("value = 0.1"), "{approved}");
    let answering = |name: &str, agree: bool| {
        let records = candidate(w.agent.path(), name, |rows| {
            for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
                let t = truth[r["item_id"].as_str().unwrap()];
                r.as_object_mut().unwrap().remove("missing");
                r["value"] = (t == agree).into();
            }
        });
        submit(&w, &records)
    };
    let out = answering("agrees.jsonl", true);
    assert_eq!(
        code(&out),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v = json(&out);
    assert_eq!(v["verdict"]["code"], "pass");
    assert_eq!(v["results"]["conditions"][0]["passed"], true);
    let out = answering("disagrees.jsonl", false);
    assert_eq!(
        code(&out),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json(&out)["results"]["conditions"][0]["passed"], false);
}

#[test]
fn the_ladder_returns_a_rounded_value_on_a_new_best_only() {
    let (referee_rows, truth) = three_raters();
    let w = world_from(&referee_rows, &gate_toml(10, 5), None);
    let approved = std::fs::read_to_string(
        w.home
            .path()
            .join("approved")
            .join(format!("{}.toml", w.approved.trim_start_matches("blake3:"))),
    )
    .unwrap();
    let approved: toml::Value = toml::from_str(&approved).unwrap();
    let step = approved["resolved"]["ladder"]["step"].as_float().unwrap();
    let first_best = approved["resolved"]["ladder"]["first_best"]
        .as_float()
        .unwrap();
    // A candidate that says what people say on every item beats the baseline.
    let perfect = |name: &str, k: u32| {
        candidate(w.agent.path(), name, |rows| {
            rows[0]["attrs"] = serde_json::json!({ "try": k });
            for r in rows.iter_mut().filter(|r| r["record"] == "judgment") {
                r.as_object_mut().unwrap().remove("missing");
                r["value"] = truth[r["item_id"].as_str().unwrap()].into();
            }
        })
    };
    let out = submit(&w, &perfect("perfect.jsonl", 1));
    let v = json(&out);
    let ladder = &v["results"]["ladder"];
    assert_eq!(ladder["metric"], "accuracy", "{v}");
    assert_eq!(ladder["step"].as_f64().unwrap(), step);
    // not_worse_than reads the difference to the baseline: 1 − first best, rounded
    // down to a multiple of the step.
    let shown = ladder["value"].as_f64().unwrap();
    let true_gap = 1.0 - first_best;
    assert!(
        shown <= true_gap && true_gap - shown < step,
        "{shown} {true_gap} {step}"
    );
    assert!(
        ((shown / step) - (shown / step).round()).abs() < 1e-9,
        "{shown} {step}"
    );
    // The best is kept for these approved conditions, private to the referee, and the
    // value is in the referee ledger.
    let best = w.home.path().join("state").join(format!(
        "{}.best.jsonl",
        w.approved.trim_start_matches("blake3:")
    ));
    assert!(best.exists(), "the best candidate is kept per approval");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&best).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let gates: Vec<Value> = referee_ledger(&w)
        .into_iter()
        .filter(|e| e["kind"] == "gate")
        .collect();
    assert!(gates.last().unwrap()["summary"]["ladder_value"].is_string());
    // The same answers again do not beat the best: no value comes back.
    let out = submit(&w, &perfect("again.jsonl", 2));
    let v = json(&out);
    assert_eq!(v["verdict"]["code"], "pass", "{v}");
    assert!(v["results"].get("ladder").is_none(), "{v}");
    // Nor does a candidate below the best.
    let out = submit(&w, &variant(w.agent.path(), 3));
    assert!(json(&out)["results"].get("ladder").is_none());
}
