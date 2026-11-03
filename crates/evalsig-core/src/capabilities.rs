//! Which analyses the data allow, which they do not, and what would make them
//! possible (SPEC §5.3 "capabilities").

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::records::{Record, ValueSemantics};

/// An analysis the data cannot support yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unavailable {
    pub analysis: &'static str,
    pub missing: &'static str,
    pub how: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    pub available: Vec<&'static str>,
    pub unavailable: Vec<Unavailable>,
}

/// The capabilities of these records, for one run or for all of them.
pub fn of(records: &[Record], run: Option<&str>) -> Capabilities {
    let in_run = |id: &str| run.is_none_or(|r| r == id);
    let runs: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            Record::Run(x) if in_run(&x.run_id) => Some(x),
            _ => None,
        })
        .collect();
    let judgments: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            Record::Judgment(j) if in_run(&j.run_id) => Some(j),
            _ => None,
        })
        .collect();
    let judged: BTreeSet<&str> = judgments.iter().map(|j| j.item_id.as_str()).collect();
    let items: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            Record::Item(i) if judged.contains(i.item_id.as_str()) => Some(i),
            _ => None,
        })
        .collect();
    let labelled = records
        .iter()
        .any(|r| matches!(r, Record::Label(l) if judged.contains(l.item_id.as_str())));
    let decision = runs
        .iter()
        .any(|r| r.value_semantics() == ValueSemantics::Decision);
    let mut per_item: BTreeMap<(&str, &str), BTreeSet<u32>> = BTreeMap::new();
    for j in &judgments {
        per_item
            .entry((j.item_id.as_str(), j.run_id.as_str()))
            .or_default()
            .insert(j.gen_replicate);
    }
    let replicated = per_item.values().any(|g| g.len() > 1);
    let clustered = items.iter().any(|i| i.cluster.is_some());
    let graded = runs.iter().all(|r| r.grader.is_some()) && !runs.is_empty();
    let inputs = items.iter().any(|i| i.input_ref.is_some());
    let timed = items.iter().any(|i| i.occurred_at.is_some());

    let mut available = Vec::new();
    let mut unavailable = Vec::new();
    let mut need = |ok: bool, names: &[&'static str], missing, how| {
        if ok {
            available.extend_from_slice(names);
        } else {
            unavailable.extend(names.iter().map(|analysis| Unavailable {
                analysis,
                missing,
                how,
            }));
        }
    };
    need(
        decision && labelled,
        &[
            "hit_rate",
            "false_alarm_rate",
            "auc",
            "d_prime",
            "agreement",
        ],
        "reference labels",
        "compare mean scores as score semantics, or import human labels from a CSV with --as label",
    );
    need(
        replicated,
        &["replicate_disagreement", "at_k"],
        "replicates",
        "run the same item two or more times (Inspect epochs, promptfoo --repeat)",
    );
    need(
        clustered,
        &["cluster_intervals"],
        "cluster",
        "map the field that names the same document, conversation or object to cluster",
    );
    need(
        graded,
        &["grader_version_check"],
        "run.grader",
        "pass the grader with import --grader",
    );
    need(
        inputs,
        &["input_changed_check"],
        "input_ref",
        "map the system-independent input (Inspect user turns, promptfoo vars) to input_ref",
    );
    need(
        timed,
        &["time_split", "trend"],
        "occurred_at",
        "map the time each item occurred to occurred_at",
    );
    if !decision {
        available.push("mean_score");
    }
    Capabilities {
        available,
        unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(text: &str) -> Vec<Record> {
        text.lines()
            .map(|l| Record::from_json(l).unwrap())
            .collect()
    }

    #[test]
    fn missing_information_names_what_to_add() {
        let rs = records(concat!(
            r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#,
            "\n",
            r#"{"record":"item","item_id":"a","cluster":"c"}"#,
            "\n",
            r#"{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"binary","value":true}"#,
            "\n",
            r#"{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":1,"output_type":"binary","value":false}"#,
        ));
        let c = of(&rs, None);
        assert!(c.available.contains(&"replicate_disagreement"));
        assert!(c.available.contains(&"cluster_intervals"));
        let missing: BTreeSet<&str> = c.unavailable.iter().map(|u| u.missing).collect();
        assert_eq!(
            missing,
            BTreeSet::from(["reference labels", "run.grader", "input_ref", "occurred_at"])
        );
        assert!(
            !of(&rs, Some("other"))
                .available
                .contains(&"replicate_disagreement")
        );
    }
}
