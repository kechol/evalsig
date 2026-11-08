//! Append-only label revisions and as-of selection (SPEC §5.1).
//!
//! A label is identified by (item, rater, task, label replicate, whether it is an
//! adjudication; D-037). Revisions append a higher `version`; nothing is overwritten.
//! Analyses read the latest version, or the latest one labelled at or before an
//! `--labels-as-of` instant.

use std::collections::BTreeMap;

use crate::records::{Label, LabelKind};
use crate::time::{Instant, TimestampError};

/// The identity of a label across its revisions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelKey {
    /// ID of the item that was labelled.
    pub item_id: String,
    /// ID of the rater who made the label.
    pub rater_id: String,
    /// The task the label belongs to.
    pub task: String,
    /// Which blind re-label of the item by this rater it is.
    pub label_replicate: u32,
    /// Whether the label adjudicates a split. A rater's adjudication is a label of its
    /// own, not a revision of that rater's vote.
    pub adjudication: bool,
}

impl LabelKey {
    /// The identity of a label.
    pub fn of(label: &Label) -> Self {
        LabelKey {
            item_id: label.item_id.clone(),
            rater_id: label.rater_id.clone(),
            task: label.task().to_owned(),
            label_replicate: label.label_replicate(),
            adjudication: label.kind() == LabelKind::Adjudication,
        }
    }
}

/// Why a revision cannot be added to a label set.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LabelError {
    /// A different record already exists under this key and version.
    #[error("label {key:?} version {version} already exists with different content")]
    Conflict {
        /// Identity of the label.
        key: LabelKey,
        /// The version that already exists.
        version: u32,
    },
    /// The revision's `labeled_at` is not a valid timestamp.
    #[error(transparent)]
    Timestamp(#[from] TimestampError),
}

/// The labels selected as of an instant.
#[derive(Debug, Default)]
pub struct AsOf<'a> {
    /// The latest revision of each label that existed at the instant.
    pub labels: Vec<&'a Label>,
    /// Labels without a `labeled_at`, which an as-of selection cannot place.
    pub undated_skipped: usize,
}

/// A revision with its `labeled_at` parsed once on insert.
#[derive(Debug, Clone)]
struct Revision {
    label: Label,
    at: Option<Instant>,
}

/// All revisions of all labels, keyed by identity and version.
#[derive(Debug, Clone, Default)]
pub struct LabelSet {
    revisions: BTreeMap<LabelKey, BTreeMap<u32, Revision>>,
}

impl LabelSet {
    /// Add a revision. Re-adding an identical record is a no-op; a different record
    /// under an existing version is rejected, since revisions never overwrite.
    pub fn insert(&mut self, label: Label) -> Result<(), LabelError> {
        let at = label
            .labeled_at
            .as_deref()
            .map(Instant::parse)
            .transpose()?;
        let versions = self.revisions.entry(LabelKey::of(&label)).or_default();
        match versions.get(&label.version) {
            Some(existing) if existing.label == label => Ok(()),
            Some(existing) => Err(LabelError::Conflict {
                key: LabelKey::of(&existing.label),
                version: label.version,
            }),
            None => {
                versions.insert(label.version, Revision { label, at });
                Ok(())
            }
        }
    }

    /// Every revision of one label, oldest version first.
    pub fn history(&self, key: &LabelKey) -> impl Iterator<Item = &Label> {
        self.revisions
            .get(key)
            .into_iter()
            .flat_map(|v| v.values().map(|r| &r.label))
    }

    /// The latest version of each label.
    pub fn latest(&self) -> Vec<&Label> {
        self.revisions
            .values()
            .filter_map(|v| v.values().next_back().map(|r| &r.label))
            .collect()
    }

    /// The latest version of each label whose `labeled_at` is at or before `as_of`.
    /// Revisions without `labeled_at` cannot be placed in time and are not selected;
    /// `undated_skipped` counts the labels whose newer revisions include an undated
    /// one (including labels with only undated revisions), so callers can warn that
    /// the as-of view may be stale.
    pub fn as_of(&self, as_of: Instant) -> AsOf<'_> {
        let mut out = AsOf::default();
        for versions in self.revisions.values() {
            let mut skipped = false;
            for r in versions.values().rev() {
                match r.at {
                    Some(t) if t <= as_of => {
                        out.labels.push(&r.label);
                        break;
                    }
                    Some(_) => {}
                    None => skipped = true,
                }
            }
            out.undated_skipped += usize::from(skipped);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{Record, Value};

    fn label(json: &str) -> Label {
        match Record::from_json(json).unwrap() {
            Record::Label(l) => l,
            _ => unreachable!(),
        }
    }

    fn alice(version: u32, value: i64, at: &str) -> Label {
        label(&format!(
            r#"{{"record":"label","item_id":"q1","rater_id":"alice","source":"human","value":{value},"levels":4,"version":{version},"labeled_at":"{at}"}}"#
        ))
    }

    #[test]
    fn revisions_never_overwrite() {
        let mut set = LabelSet::default();
        set.insert(alice(1, 3, "2026-08-03T00:00:00Z")).unwrap();
        set.insert(alice(2, 2, "2026-09-10T00:00:00Z")).unwrap();
        set.insert(alice(1, 3, "2026-08-03T00:00:00Z")).unwrap();
        let err = set.insert(alice(1, 1, "2026-08-03T00:00:00Z")).unwrap_err();
        assert!(matches!(err, LabelError::Conflict { version: 1, .. }));
        let key = LabelKey::of(&alice(1, 3, "2026-08-03T00:00:00Z"));
        let values: Vec<_> = set.history(&key).map(|l| l.value).collect();
        assert_eq!(values, [Some(Value::Int(3)), Some(Value::Int(2))]);
    }

    #[test]
    fn latest_is_the_default() {
        let mut set = LabelSet::default();
        set.insert(alice(2, 2, "2026-09-10T00:00:00Z")).unwrap();
        set.insert(alice(1, 3, "2026-08-03T00:00:00Z")).unwrap();
        let latest = set.latest();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].version, 2);
    }

    #[test]
    fn as_of_selects_the_version_valid_at_that_time() {
        let mut set = LabelSet::default();
        set.insert(alice(1, 3, "2026-08-03T00:00:00Z")).unwrap();
        set.insert(alice(2, 2, "2026-09-10T00:00:00Z")).unwrap();
        let at = |s| set.as_of(Instant::parse(s).unwrap()).labels;
        assert!(at("2026-08-01T00:00:00Z").is_empty());
        assert_eq!(at("2026-08-03T00:00:00Z")[0].version, 1);
        assert_eq!(at("2026-09-09T23:59:59Z")[0].version, 1);
        assert_eq!(at("2026-09-10T09:00:00+09:00")[0].version, 2);
        assert_eq!(at("2027-01-01T00:00:00Z")[0].version, 2);
    }

    #[test]
    fn keys_separate_raters_tasks_and_replicates() {
        let mut set = LabelSet::default();
        set.insert(alice(1, 3, "2026-08-03T00:00:00Z")).unwrap();
        set.insert(label(r#"{"record":"label","item_id":"q1","rater_id":"bob","source":"human","value":2,"version":1}"#)).unwrap();
        set.insert(label(r#"{"record":"label","item_id":"q1","rater_id":"alice","source":"human","label_replicate":1,"value":2,"version":1}"#)).unwrap();
        set.insert(label(r#"{"record":"label","item_id":"q1","rater_id":"alice","source":"human","task":"tone","value":0,"version":1}"#)).unwrap();
        assert_eq!(set.latest().len(), 4);
        // Undated revisions are never selected as of a time.
        let as_of = set.as_of(Instant::parse("2027-01-01T00:00:00Z").unwrap());
        assert_eq!(as_of.labels.len(), 1);
        assert_eq!(as_of.undated_skipped, 3);
    }

    #[test]
    fn an_adjudication_stands_beside_the_raters_vote() {
        // Alice votes 3, then adjudicates the split as 1 under her own rater ID: two
        // labels, at version 1 and at version 2 alike (D-037).
        for version in [1, 2] {
            let mut set = LabelSet::default();
            set.insert(alice(1, 3, "2026-08-03T00:00:00Z")).unwrap();
            set.insert(label(&format!(
                r#"{{"record":"label","item_id":"q1","rater_id":"alice","source":"human","kind":"adjudication","value":1,"levels":4,"version":{version}}}"#
            )))
            .unwrap();
            let mut latest: Vec<_> = set.latest().iter().map(|l| (l.kind(), l.value)).collect();
            latest.sort_by_key(|(k, _)| *k == LabelKind::Adjudication);
            assert_eq!(
                latest,
                [
                    (LabelKind::Independent, Some(Value::Int(3))),
                    (LabelKind::Adjudication, Some(Value::Int(1)))
                ]
            );
        }
        // A revision of the vote stays the same label.
        let mut set = LabelSet::default();
        set.insert(alice(1, 3, "2026-08-03T00:00:00Z")).unwrap();
        set.insert(label(r#"{"record":"label","item_id":"q1","rater_id":"alice","source":"human","kind":"revision","value":2,"levels":4,"version":2}"#)).unwrap();
        assert_eq!(set.latest().len(), 1);
    }

    #[test]
    fn a_newer_undated_revision_is_reported() {
        let mut set = LabelSet::default();
        set.insert(alice(1, 3, "2026-08-03T00:00:00Z")).unwrap();
        set.insert(label(r#"{"record":"label","item_id":"q1","rater_id":"alice","source":"human","value":2,"levels":4,"version":2}"#)).unwrap();
        let as_of = set.as_of(Instant::parse("2027-01-01T00:00:00Z").unwrap());
        assert_eq!(as_of.labels[0].version, 1);
        assert_eq!(as_of.undated_skipped, 1);
        assert_eq!(set.latest()[0].version, 2);
    }
}
