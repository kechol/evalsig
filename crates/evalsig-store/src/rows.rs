//! Primary-key order of canonical records in a snapshot file (audit spec §1).
//!
//! Records sort by kind (run, item, label, judgment, preference, content), then by the
//! kind's key fields. Strings compare by Unicode code point, which is UTF-8 byte order.

use std::cmp::Ordering;

use evalsig_core::records::{LabelKind, Record};

use crate::canonical::{CanonicalError, to_canonical_bytes};
use crate::hash::{ContentHash, file_hash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Part<'a> {
    None,
    Str(&'a str),
    Num(u32),
}

/// The kind rank and up to eight key fields, padded with `Part::None`; built
/// without allocating, since sorting calls it for every comparison.
fn key(r: &Record) -> (u8, [Part<'_>; 8]) {
    use Part::{None as N, Num, Str};
    match r {
        Record::Run(x) => (0, [Str(&x.run_id), N, N, N, N, N, N, N]),
        Record::Item(x) => (1, [Str(&x.item_id), N, N, N, N, N, N, N]),
        Record::Label(x) => (
            2,
            [
                Str(&x.item_id),
                Str(&x.rater_id),
                Str(x.task()),
                Num(x.label_replicate()),
                // An adjudication is a label of its own beside the rater's vote (D-037).
                Num(u32::from(x.kind() == LabelKind::Adjudication)),
                Num(x.version),
                N,
                N,
            ],
        ),
        Record::Judgment(x) => (
            3,
            [
                Str(&x.item_id),
                Str(&x.run_id),
                Str(x.task()),
                Num(x.gen_replicate),
                Num(x.grade_replicate()),
                N,
                N,
                N,
            ],
        ),
        Record::Preference(x) => (
            4,
            [
                Str(&x.item_id),
                Str(x.task()),
                Str(&x.left_run_id),
                Str(&x.right_run_id),
                Num(x.left_gen_replicate.unwrap_or(0)),
                Num(x.right_gen_replicate.unwrap_or(0)),
                Str(&x.rater_id),
                Num(x.grade_replicate.unwrap_or(0)),
            ],
        ),
        Record::Content(x) => (5, [Str(&x.content_ref), N, N, N, N, N, N, N]),
    }
}

const KEY_NAMES: [&[&str]; 6] = [
    &["run_id"],
    &["item_id"],
    &[
        "item_id",
        "rater_id",
        "task",
        "label_replicate",
        "adjudication",
        "version",
    ],
    &[
        "item_id",
        "run_id",
        "task",
        "gen_replicate",
        "grade_replicate",
    ],
    &[
        "item_id",
        "task",
        "left_run_id",
        "right_run_id",
        "left_gen_replicate",
        "right_gen_replicate",
        "rater_id",
        "grade_replicate",
    ],
    &["ref"],
];

/// Primary-key order of two records.
pub fn compare(a: &Record, b: &Record) -> Ordering {
    key(a).cmp(&key(b))
}

/// Sort records into primary-key order.
pub fn sort(records: &mut [Record]) {
    records.sort_by(compare);
}

/// The primary key as `kind{field=value, …}`, for messages.
pub fn describe_key(r: &Record) -> String {
    let (rank, parts) = key(r);
    let kind = ["run", "item", "label", "judgment", "preference", "content"][rank as usize];
    let fields: Vec<String> = KEY_NAMES[rank as usize]
        .iter()
        .zip(parts)
        .map(|(name, p)| match p {
            Part::Str(s) => format!("{name}={s:?}"),
            Part::Num(n) => format!("{name}={n}"),
            Part::None => String::new(),
        })
        .collect();
    format!("{kind}{{{}}}", fields.join(", "))
}

/// Sort, merge identical records, and return the keys that two different records
/// share, which a single snapshot must not contain.
pub fn dedup_and_conflicts(records: &mut Vec<Record>) -> Vec<String> {
    sort(records);
    records.dedup();
    records
        .windows(2)
        .filter(|w| compare(&w[0], &w[1]).is_eq())
        .map(|w| describe_key(&w[1]))
        .collect()
}

/// Layer snapshots given in import order: for each primary key the record from the
/// latest snapshot wins. The result is in primary-key order.
pub fn layer(snapshots: Vec<Vec<Record>>) -> Vec<Record> {
    let mut tagged: Vec<(usize, Record)> = snapshots
        .into_iter()
        .enumerate()
        .flat_map(|(i, rs)| rs.into_iter().map(move |r| (i, r)))
        .collect();
    tagged.sort_by(|a, b| compare(&a.1, &b.1).then(a.0.cmp(&b.0)));
    let mut out: Vec<Record> = Vec::with_capacity(tagged.len());
    for (_, r) in tagged {
        match out.last_mut() {
            Some(last) if compare(last, &r).is_eq() => *last = r,
            _ => out.push(r),
        }
    }
    out
}

/// Canonical lines of records in primary-key order, and the snapshot file hash.
pub fn canonical_lines(
    records: &mut [Record],
) -> Result<(Vec<Vec<u8>>, ContentHash), CanonicalError> {
    sort(records);
    let lines = records
        .iter()
        .map(to_canonical_bytes)
        .collect::<Result<Vec<_>, _>>()?;
    let hash = file_hash(&lines);
    Ok((lines, hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLES: &str = include_str!("../../../examples/records.jsonl");

    fn examples() -> Vec<Record> {
        EXAMPLES
            .lines()
            .map(|l| Record::from_json(l).unwrap())
            .collect()
    }

    #[test]
    fn order_does_not_depend_on_input_order() {
        let mut a = examples();
        let mut b = examples();
        b.reverse();
        let (la, ha) = canonical_lines(&mut a).unwrap();
        let (lb, hb) = canonical_lines(&mut b).unwrap();
        assert_eq!(la, lb);
        assert_eq!(ha, hb);
    }

    #[test]
    fn kinds_then_keys() {
        let mut r = examples();
        sort(&mut r);
        let kinds: Vec<u8> = r.iter().map(|x| key(x).0).collect();
        assert!(kinds.windows(2).all(|w| w[0] <= w[1]));
        assert!(matches!(r[0], Record::Run(_)));
        assert!(matches!(r.last(), Some(Record::Content(_))));
        let versions: Vec<u32> = r
            .iter()
            .filter_map(|x| match x {
                Record::Label(l) if l.rater_id == "user:alice" && l.item_id == "q-0001" => {
                    Some(l.version)
                }
                _ => None,
            })
            .collect();
        // Replicate 0 versions 1 and 2, then replicate 1 version 1.
        assert_eq!(versions, [1, 2, 1]);
    }

    #[test]
    fn keys_describe_and_conflicts() {
        let r = Record::from_json(r#"{"record":"label","item_id":"q","rater_id":"u","source":"human","version":2,"value":true}"#).unwrap();
        assert_eq!(
            describe_key(&r),
            r#"label{item_id="q", rater_id="u", task="default", label_replicate=0, adjudication=0, version=2}"#
        );
        let other = Record::from_json(r#"{"record":"label","item_id":"q","rater_id":"u","source":"human","version":2,"value":false}"#).unwrap();
        let mut rs = vec![r.clone(), r.clone(), other];
        assert_eq!(dedup_and_conflicts(&mut rs).len(), 1);
        assert_eq!(rs.len(), 2);
    }

    #[test]
    fn later_snapshots_win() {
        let a =
            Record::from_json(r#"{"record":"run","run_id":"r","system":{"id":"s","version":"1"}}"#)
                .unwrap();
        let b =
            Record::from_json(r#"{"record":"run","run_id":"r","system":{"id":"s","version":"2"}}"#)
                .unwrap();
        let c = Record::from_json(r#"{"record":"item","item_id":"i"}"#).unwrap();
        let out = layer(vec![vec![a, c.clone()], vec![b.clone()]]);
        assert_eq!(out, [b, c]);
    }

    #[test]
    fn integer_values_stay_integers_and_float_ones_lose_the_fraction() {
        let r = Record::from_json(r#"{"record":"item","item_id":"a","weight":2.0}"#).unwrap();
        assert_eq!(
            to_canonical_bytes(&r).unwrap(),
            br#"{"item_id":"a","record":"item","weight":2}"#
        );
    }
}
