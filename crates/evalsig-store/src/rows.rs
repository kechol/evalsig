//! Primary-key order of canonical records in a snapshot file (audit spec §1).
//!
//! Records sort by kind (run, item, label, judgment, preference, content), then by the
//! kind's key fields. Strings compare by Unicode code point, which is UTF-8 byte order.

use std::cmp::Ordering;

use evalsig_core::records::Record;

use crate::canonical::{CanonicalError, to_canonical_bytes};
use crate::hash::{ContentHash, file_hash};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Part<'a> {
    Str(&'a str),
    Num(u32),
}

fn key(r: &Record) -> (u8, Vec<Part<'_>>) {
    use Part::{Num, Str};
    match r {
        Record::Run(x) => (0, vec![Str(&x.run_id)]),
        Record::Item(x) => (1, vec![Str(&x.item_id)]),
        Record::Label(x) => (
            2,
            vec![
                Str(&x.item_id),
                Str(&x.rater_id),
                Str(x.task()),
                Num(x.label_replicate()),
                Num(x.version),
            ],
        ),
        Record::Judgment(x) => (
            3,
            vec![
                Str(&x.item_id),
                Str(&x.run_id),
                Str(x.task()),
                Num(x.gen_replicate),
                Num(x.grade_replicate()),
            ],
        ),
        Record::Preference(x) => (
            4,
            vec![
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
        Record::Content(x) => (5, vec![Str(&x.content_ref)]),
    }
}

/// Primary-key order of two records.
pub fn compare(a: &Record, b: &Record) -> Ordering {
    key(a).cmp(&key(b))
}

/// Sort records into primary-key order.
pub fn sort(records: &mut [Record]) {
    records.sort_by(compare);
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
    fn integer_values_stay_integers_and_float_ones_lose_the_fraction() {
        let r = Record::from_json(r#"{"record":"item","item_id":"a","weight":2.0}"#).unwrap();
        assert_eq!(
            to_canonical_bytes(&r).unwrap(),
            br#"{"item_id":"a","record":"item","weight":2}"#
        );
    }
}
