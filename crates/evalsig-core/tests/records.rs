use evalsig_core::records::{MissingReason, Observation, OutputType, Record, RecordError, Value};
use serde_json::Value as Json;

const EXAMPLES: &str = include_str!("../../../examples/records.jsonl");

/// JSON equality that treats 2 and 2.0 as the same number, as canonical JSON does.
fn same(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64() == y.as_f64(),
        (Json::Array(x), Json::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(u, v)| same(u, v))
        }
        (Json::Object(x), Json::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

#[test]
fn every_example_record_round_trips_unchanged() {
    let mut n = 0;
    for line in EXAMPLES.lines().filter(|l| !l.trim().is_empty()) {
        let record = Record::from_json(line).unwrap_or_else(|e| panic!("{e}: {line}"));
        let back = serde_json::to_value(&record).unwrap();
        let original: Json = serde_json::from_str(line).unwrap();
        assert!(same(&original, &back), "changed:\n{original}\n{back}");
        n += 1;
    }
    assert_eq!(n, 25);
}

#[test]
fn integers_and_floats_keep_their_form() {
    let r = Record::from_json(
        r#"{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"score","value":1}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&r)
            .unwrap()
            .matches("\"value\":1}")
            .count(),
        1
    );
}

#[test]
fn unknown_fields_and_records_are_rejected() {
    for line in [
        r#"{"record":"item","item_id":"a","colour":"red"}"#,
        r#"{"record":"verdict","item_id":"a"}"#,
        r#"{"item_id":"a"}"#,
    ] {
        assert!(
            matches!(Record::from_json(line), Err(RecordError::Json(_))),
            "{line}"
        );
    }
}

#[test]
fn value_and_missing_are_exclusive() {
    let both = r#"{"record":"label","item_id":"a","rater_id":"u","source":"human","version":1,"value":true,"missing":{"reason":"other"}}"#;
    let neither = r#"{"record":"label","item_id":"a","rater_id":"u","source":"human","version":1}"#;
    assert!(matches!(
        Record::from_json(both),
        Err(RecordError::ValueAndMissing)
    ));
    assert!(matches!(
        Record::from_json(neither),
        Err(RecordError::NeitherValueNorMissing)
    ));
    let pref = r#"{"record":"preference","item_id":"a","left_run_id":"x","right_run_id":"y","rater_id":"u","source":"model"}"#;
    assert!(matches!(
        Record::from_json(pref),
        Err(RecordError::NeitherValueNorMissing)
    ));
}

#[test]
fn all_ten_missing_reasons_parse() {
    for reason in MissingReason::ALL {
        let code = serde_json::to_string(&reason).unwrap();
        let line = format!(
            r#"{{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"binary","missing":{{"reason":{code}}}}}"#
        );
        let Record::Judgment(j) = Record::from_json(&line).unwrap() else {
            panic!()
        };
        assert!(matches!(j.observation().unwrap(), Observation::Missing(m) if m.reason == reason));
    }
    let bad = r#"{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"binary","missing":{"reason":"nap"}}"#;
    assert!(Record::from_json(bad).is_err());
}

#[test]
fn values_must_match_the_output_type() {
    let j = |t: &str, extra: &str, v: &str| {
        format!(
            r#"{{"record":"judgment","item_id":"a","run_id":"r","gen_replicate":0,"output_type":"{t}"{extra},"value":{v}}}"#
        )
    };
    assert!(Record::from_json(&j("binary", "", "true")).is_ok());
    assert!(matches!(
        Record::from_json(&j("binary", "", "1")),
        Err(RecordError::ValueType {
            output_type: OutputType::Binary,
            value: Value::Int(1)
        })
    ));
    assert!(Record::from_json(&j("ordinal", r#","levels":4"#, "3")).is_ok());
    assert!(matches!(
        Record::from_json(&j("ordinal", r#","levels":4"#, "4")),
        Err(RecordError::LevelOutOfRange {
            value: 4,
            levels: 4
        })
    ));
    assert!(matches!(
        Record::from_json(&j("ordinal", r#","levels":4"#, "-1")),
        Err(RecordError::LevelOutOfRange { .. })
    ));
    assert!(matches!(
        Record::from_json(&j("ordinal", "", "1")),
        Err(RecordError::MissingLevels)
    ));
    assert!(matches!(
        Record::from_json(&j("ordinal", r#","levels":4"#, "1.5")),
        Err(RecordError::ValueType { .. })
    ));
    assert!(matches!(
        Record::from_json(&j("ordinal", r#","levels":1"#, "0")),
        Err(RecordError::TooFewLevels)
    ));
    assert!(Record::from_json(&j("score", "", "0.71")).is_ok());
    assert!(Record::from_json(&j("score", "", "false")).is_err());
}

#[test]
fn defaults_come_from_accessors() {
    let Record::Item(item) = Record::from_json(r#"{"record":"item","item_id":"q"}"#).unwrap()
    else {
        panic!()
    };
    assert_eq!(item.weight(), 1.0);
    assert_eq!(item.cluster_key(), "q");
    let Record::Run(run) =
        Record::from_json(r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"}}"#)
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        run.value_semantics(),
        evalsig_core::records::ValueSemantics::Decision
    );
}

#[test]
fn non_finite_scores_become_missing() {
    assert_eq!(Value::from_score(0.5), Ok(Value::Float(0.5)));
    for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(Value::from_score(x), Err(MissingReason::InvalidOutput));
    }
}

#[test]
fn timestamps_must_be_rfc3339() {
    let bad = r#"{"record":"item","item_id":"a","occurred_at":"yesterday"}"#;
    let err = Record::from_json(bad).unwrap_err();
    assert_eq!(err.code(), "record.timestamp");
    assert!(
        Record::from_json(
            r#"{"record":"item","item_id":"a","occurred_at":"2026-08-01T00:00:00Z"}"#
        )
        .is_ok()
    );
}
