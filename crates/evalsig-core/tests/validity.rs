//! Each validity check of the M1 data model: one fixture that triggers it and one that
//! does not.

use std::collections::BTreeMap;

use evalsig_core::checks::{Action, Finding, Overrides};
use evalsig_core::records::{GraderRef, Label, Record};
use evalsig_core::validity::{
    agreement_labels, dataset, missing_rate_differs, mixed_grader, pair,
    run_variance_exceeds_delta, secret_shape,
};

fn recs(lines: &[&str]) -> Vec<Record> {
    lines
        .iter()
        .map(|l| Record::from_json(l).unwrap())
        .collect()
}

fn codes(findings: &[Finding]) -> Vec<&'static str> {
    findings.iter().map(|f| f.code).collect()
}

fn fired(records: &[Record], code: &str) -> Option<Finding> {
    dataset(records, &Overrides::default())
        .into_iter()
        .find(|f| f.code == code)
}

const RUN: &str =
    r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"},"output_type":"binary"}"#;

fn judgment(item: &str, rep: u32, extra: &str) -> String {
    format!(
        r#"{{"record":"judgment","item_id":"{item}","run_id":"r","gen_replicate":{rep},"output_type":"binary"{extra}}}"#
    )
}

#[test]
fn unexplained_missing() {
    let bad = recs(&[RUN, &judgment("a", 0, r#","missing":{"reason":"other"}"#)]);
    let f = fired(&bad, "unexplained_missing").unwrap();
    assert_eq!(f.action, Action::Stop);
    let lifted = dataset(
        &bad,
        &Overrides {
            allow_unexplained_missing: true,
            ..Overrides::default()
        },
    );
    assert_eq!(
        lifted
            .iter()
            .find(|f| f.code == "unexplained_missing")
            .unwrap()
            .action,
        Action::Warn
    );
    let ok = recs(&[
        RUN,
        &judgment(
            "a",
            0,
            r#","missing":{"reason":"other","detail":"provider returned 503"}"#,
        ),
    ]);
    assert!(fired(&ok, "unexplained_missing").is_none());
    let ok = recs(&[RUN, &judgment("a", 0, r#","missing":{"reason":"timeout"}"#)]);
    assert!(fired(&ok, "unexplained_missing").is_none());
}

#[test]
fn mixed_output_type() {
    let bad = recs(&[
        RUN,
        &judgment("a", 0, r#","value":true"#),
        r#"{"record":"judgment","item_id":"b","run_id":"r","gen_replicate":0,"output_type":"score","value":0.4}"#,
    ]);
    assert!(fired(&bad, "mixed_output_type").unwrap().stops());
    let levels = recs(&[
        r#"{"record":"judgment","item_id":"a","run_id":"q","gen_replicate":0,"output_type":"ordinal","levels":4,"value":1}"#,
        r#"{"record":"judgment","item_id":"b","run_id":"q","gen_replicate":0,"output_type":"ordinal","levels":5,"value":1}"#,
    ]);
    assert!(fired(&levels, "mixed_output_type").is_some());
    let ok = recs(&[
        RUN,
        &judgment("a", 0, r#","value":true"#),
        &judgment("b", 0, r#","value":false"#),
    ]);
    assert!(fired(&ok, "mixed_output_type").is_none());
}

#[test]
fn identical_replicates() {
    let h = "blake3:".to_owned() + &"1".repeat(64);
    let g = "blake3:".to_owned() + &"2".repeat(64);
    let out = |r: &str| format!(r#","value":true,"output_ref":"{r}""#);
    let bad = recs(&[
        RUN,
        &judgment("a", 0, &out(&h)),
        &judgment("a", 1, &out(&h)),
    ]);
    assert_eq!(fired(&bad, "identical_replicates").unwrap().count, Some(1));
    let ok = recs(&[
        RUN,
        &judgment("a", 0, &out(&h)),
        &judgment("a", 1, &out(&g)),
    ]);
    assert!(fired(&ok, "identical_replicates").is_none());
}

fn label(item: &str, extra: &str) -> String {
    format!(
        r#"{{"record":"label","item_id":"{item}","rater_id":"u","source":"human","version":1,"value":true{extra}}}"#
    )
}

/// Twenty items, ten judged true and ten false; `labelled(i)` says which get a label.
fn verification_fixture(labelled: impl Fn(usize) -> bool) -> Vec<Record> {
    let mut lines = vec![RUN.to_owned()];
    for i in 0..20 {
        lines.push(format!(r#"{{"record":"item","item_id":"i{i}"}}"#));
        lines.push(judgment(
            &format!("i{i}"),
            0,
            &format!(r#","value":{}"#, i < 10),
        ));
        if labelled(i) {
            lines.push(label(&format!("i{i}"), ""));
        }
    }
    recs(&lines.iter().map(String::as_str).collect::<Vec<_>>())
}

#[test]
fn verification_bias_suspected() {
    // Humans labelled every item judged true and none judged false.
    assert!(
        fired(
            &verification_fixture(|i| i < 10),
            "verification_bias_suspected"
        )
        .is_some()
    );
    // Labels spread evenly over both levels.
    assert!(
        fired(
            &verification_fixture(|i| i % 2 == 0),
            "verification_bias_suspected"
        )
        .is_none()
    );
    // Every item labelled: nothing was selected.
    assert!(
        fired(
            &verification_fixture(|_| true),
            "verification_bias_suspected"
        )
        .is_none()
    );
}

#[test]
fn model_derived_reference() {
    let bad = recs(&[
        r#"{"record":"label","item_id":"a","rater_id":"model:j","source":"model","version":1,"value":true}"#,
    ]);
    assert_eq!(
        fired(&bad, "model_derived_reference").unwrap().action,
        Action::Warn
    );
    let ok = recs(&[
        r#"{"record":"label","item_id":"a","rater_id":"u","source":"human","version":1,"value":true}"#,
    ]);
    assert!(fired(&ok, "model_derived_reference").is_none());
}

#[test]
fn secret_in_content() {
    let key = format!("AKIA{}", "Q".repeat(16));
    let bad = recs(&[&format!(
        r#"{{"record":"label","item_id":"a","rater_id":"u","source":"human","version":1,"value":true,"reason":"pasted {key} by mistake"}}"#
    )]);
    let warn = fired(&bad, "secret_in_content").unwrap();
    assert_eq!(warn.action, Action::Warn);
    let stop = dataset(
        &bad,
        &Overrides {
            fail_on_secret: true,
            ..Overrides::default()
        },
    );
    assert!(
        stop.iter()
            .any(|f| f.code == "secret_in_content" && f.stops())
    );
    let ok = recs(&[
        r#"{"record":"label","item_id":"a","rater_id":"u","source":"human","version":1,"value":true,"reason":"ask about the sk-ip rule"}"#,
    ]);
    assert!(fired(&ok, "secret_in_content").is_none());
    assert_eq!(
        secret_shape("-----BEGIN RSA PRIVATE KEY-----\nMII"),
        Some("private key")
    );
    assert_eq!(
        secret_shape(&format!("token ghp_{}", "a".repeat(36))),
        Some("GitHub token")
    );
    assert_eq!(secret_shape("an ordinary sentence"), None);
    let run = recs(&[&format!(
        r#"{{"record":"run","run_id":"r","system":{{"id":"s","version":"1"}},"harness":{{"name":"h","run_ref":"xoxp-{}"}}}}"#,
        "1".repeat(24)
    )]);
    assert!(fired(&run, "secret_in_content").is_some());
}

#[test]
fn adjudication_never_enters_agreement() {
    let labels: Vec<Label> = recs(&[
        &label("a", ""),
        r#"{"record":"label","item_id":"a","rater_id":"lead","source":"human","kind":"adjudication","version":1,"value":true}"#,
    ])
    .into_iter()
    .filter_map(|r| if let Record::Label(l) = r { Some(l) } else { None })
    .collect();
    let kept = agreement_labels(&labels);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].rater_id, "u");
}

#[test]
fn mixed_grader_stops() {
    let g = |v: &str| GraderRef {
        id: Some("judge".into()),
        version: Some(v.into()),
        ..GraderRef::default()
    };
    let f = mixed_grader(Some(&g("1")), Some(&g("2")), &Overrides::default()).unwrap();
    assert!(f.stops());
    let lifted = mixed_grader(
        Some(&g("1")),
        Some(&g("2")),
        &Overrides {
            allow_mixed_grader: true,
            ..Overrides::default()
        },
    );
    assert_eq!(lifted.unwrap().action, Action::Warn);
    assert!(mixed_grader(Some(&g("1")), Some(&g("1")), &Overrides::default()).is_none());
    assert!(mixed_grader(None, None, &Overrides::default()).is_none());
    let one = mixed_grader(Some(&g("1")), None, &Overrides::default()).unwrap();
    assert!(one.message.contains("only one arm"));
    assert!(f.message.contains("version"));
}

#[test]
fn unpaired_items_and_changed_inputs() {
    let a: BTreeMap<&str, Option<&str>> =
        [("x", Some("h1")), ("y", Some("h2")), ("z", None)].into();
    let b: BTreeMap<&str, Option<&str>> =
        [("x", Some("h1")), ("y", Some("h9")), ("w", None)].into();
    let (p, findings) = pair(&a, &b, &Overrides::default());
    assert_eq!(p.paired, ["x"]);
    assert_eq!(p.n_unpaired, 2);
    assert_eq!(p.input_changed, ["y"]);
    assert_eq!(codes(&findings), ["unpaired_items", "input_changed"]);
    assert_eq!(findings[1].action, Action::Exclude);
    let (p, findings) = pair(
        &a,
        &b,
        &Overrides {
            allow_input_change: true,
            ..Overrides::default()
        },
    );
    assert_eq!(p.paired, ["x", "y"]);
    assert_eq!(findings[1].action, Action::Warn);
    let (p, findings) = pair(&a, &a, &Overrides::default());
    assert_eq!(p.paired.len(), 3);
    assert!(findings.is_empty());
}

#[test]
fn missing_rates_that_differ() {
    assert!(missing_rate_differs((30, 100), (5, 100), &Overrides::default()).is_some());
    assert!(missing_rate_differs((6, 100), (5, 100), &Overrides::default()).is_none());
}

#[test]
fn run_variance_against_delta() {
    assert!(run_variance_exceeds_delta(0.02, 0.01, &Overrides::default()).is_some());
    assert!(run_variance_exceeds_delta(0.005, -0.01, &Overrides::default()).is_none());
}

#[test]
fn blank_details_do_not_explain_a_missing_value() {
    let blank = recs(&[
        RUN,
        &judgment("a", 0, r#","missing":{"reason":"other","detail":"  "}"#),
    ]);
    assert!(fired(&blank, "unexplained_missing").is_some());
}

#[test]
fn missing_judgments_without_levels_are_not_mixed_shapes() {
    let ordinal = recs(&[
        r#"{"record":"run","run_id":"q","system":{"id":"s","version":"1"},"output_type":"ordinal","levels":4}"#,
        r#"{"record":"judgment","item_id":"a","run_id":"q","gen_replicate":0,"output_type":"ordinal","levels":4,"value":1}"#,
        r#"{"record":"judgment","item_id":"b","run_id":"q","gen_replicate":0,"output_type":"ordinal","missing":{"reason":"timeout"}}"#,
    ]);
    assert!(fired(&ordinal, "mixed_output_type").is_none());
}
