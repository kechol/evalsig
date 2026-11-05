//! Building one reference from several raters' labels (metrics spec §9.2).
//!
//! Under `soft` every metric is the value against one rater drawn at random: an item
//! with positive share s_i counts s_i toward the positive side and 1 − s_i toward the
//! negative side. `single` and `majority` give shares of 0, ½ (a tie) or 1.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::agreement;
use crate::config::{RaterDesign, ReferenceMode};
use crate::records::{Label, LabelKind, Source};
use crate::rules::Rule;

/// The reference of one item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reference {
    /// The share of the reference that is positive.
    pub share: f64,
    /// Whether only model raters produced it.
    pub model_derived: bool,
}

/// The labels a reference is built from: the human ones when there are any. Also
/// whether only model raters produced them.
fn pool<'l>(labels: &[&'l Label]) -> (Vec<&'l Label>, bool) {
    let human: Vec<&Label> = labels
        .iter()
        .copied()
        .filter(|l| l.source == Source::Human)
        .collect();
    let pool = if human.is_empty() {
        labels.to_vec()
    } else {
        human
    };
    let model_derived = pool.iter().all(|l| l.source == Source::Model);
    (pool, model_derived)
}

/// Each rater's vote on one item: their first label replicate, read as positive or
/// not. Adjudications are not votes.
pub fn votes<'l>(labels: &[&'l Label], positive: &Rule) -> Vec<(&'l str, bool)> {
    votes_in(&pool(labels).0, positive)
}

/// The votes among labels already pooled.
fn votes_in<'l>(pool: &[&'l Label], positive: &Rule) -> Vec<(&'l str, bool)> {
    pool.iter()
        .filter(|l| l.kind() != LabelKind::Adjudication && l.label_replicate() == 0)
        .filter_map(|l| {
            l.value
                .and_then(|v| positive.is_positive(v))
                .map(|p| (l.rater_id.as_str(), p))
        })
        .collect()
}

/// The stratum `reference_agreement` (metrics spec §9.6).
pub const AGREEMENT_STRATUM: &str = "reference_agreement";

/// `unanimous` or `split` for an item that two or more human raters voted on.
pub fn agreement_stratum(labels: &[&Label], positive: &Rule) -> Option<&'static str> {
    let human: Vec<&Label> = labels
        .iter()
        .copied()
        .filter(|l| l.source == Source::Human)
        .collect();
    let votes = votes_in(&human, positive);
    let first = votes.first()?.1;
    (votes.len() >= 2).then(|| {
        if votes.iter().all(|(_, v)| *v == first) {
            "unanimous"
        } else {
            "split"
        }
    })
}

/// The reference of one item from the latest revision of its labels. Human labels
/// are used when there are any; adjudications decide under `single` and are left out
/// of the votes otherwise, as are later label replicates of the same rater.
pub fn of(labels: &[&Label], positive: &Rule, mode: ReferenceMode) -> Option<Reference> {
    let (pool, model_derived) = pool(labels);
    if mode == ReferenceMode::Single
        && let Some(adj) = pool.iter().find(|l| l.kind() == LabelKind::Adjudication)
    {
        return adj
            .value
            .and_then(|v| positive.is_positive(v))
            .map(|p| Reference {
                share: f64::from(u8::from(p)),
                model_derived,
            });
    }
    let votes = votes_in(&pool, positive);
    if votes.is_empty() {
        return None;
    }
    let share = votes.iter().filter(|(_, v)| *v).count() as f64 / votes.len() as f64;
    let share = match mode {
        ReferenceMode::Soft => share,
        // Majority, and single without an adjudication: ties count one half.
        _ => match share.partial_cmp(&0.5) {
            Some(std::cmp::Ordering::Greater) => 1.0,
            Some(std::cmp::Ordering::Less) => 0.0,
            _ => 0.5,
        },
    };
    Some(Reference {
        share,
        model_derived,
    })
}

/// How raters were assigned to items (metrics spec §9.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Design {
    /// Different raters on different items.
    Nested,
    /// The same few raters on most items.
    Crossed,
}

impl Design {
    /// What the intervals estimate under this design.
    pub fn estimand(self) -> &'static str {
        match self {
            Design::Nested => "performance against one rater drawn at random",
            Design::Crossed => "performance against these raters",
        }
    }
}

/// The rater design of a reference and what follows from it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesignInfo {
    /// Whether raters are nested or crossed.
    pub rater_design: Design,
    /// Whether the design was detected from the overlap rather than set.
    pub detected: bool,
    /// What the reported performance is measured against.
    pub estimand: &'static str,
    /// The raters whose votes form the reference.
    pub voters: usize,
    /// How many items have each number of raters.
    pub raters_per_item: BTreeMap<usize, usize>,
}

/// Below this share of the items per rater (the median rater), the design is nested.
const NESTED_SHARE: f64 = 0.2;

/// The design from the raters who voted on each item, unless `setting` fixes it.
/// `None` when no item has a vote.
pub fn design<'r>(
    voters: impl IntoIterator<Item = BTreeSet<&'r str>>,
    setting: RaterDesign,
) -> Option<DesignInfo> {
    let mut per_rater: BTreeMap<&str, usize> = BTreeMap::new();
    let mut raters_per_item = BTreeMap::new();
    let mut items = 0;
    for v in voters.into_iter().filter(|v| !v.is_empty()) {
        items += 1;
        *raters_per_item.entry(v.len()).or_insert(0) += 1;
        for r in v {
            *per_rater.entry(r).or_insert(0) += 1;
        }
    }
    if items == 0 {
        return None;
    }
    let mut counts: Vec<usize> = per_rater.values().copied().collect();
    counts.sort_unstable();
    let m = counts.len();
    let median = (counts[(m - 1) / 2] + counts[m / 2]) as f64 / 2.0;
    let (rater_design, detected) = match setting {
        RaterDesign::Auto if median < NESTED_SHARE * items as f64 => (Design::Nested, true),
        RaterDesign::Auto => (Design::Crossed, true),
        RaterDesign::Nested => (Design::Nested, false),
        RaterDesign::Crossed => (Design::Crossed, false),
    };
    Some(DesignInfo {
        rater_design,
        detected,
        estimand: rater_design.estimand(),
        voters: m,
        raters_per_item,
    })
}

/// The accuracy no system can exceed against soft references, mean(max(s_i, 1 − s_i))
/// (metrics spec §9.1).
pub fn accuracy_ceiling(shares: &[f64]) -> Option<f64> {
    crate::numeric::mean(shares.iter().map(|s| s.max(1.0 - s)))
}

/// `kappa_vs_reference` under `soft`: Cohen's κ of the system's decisions against
/// each rater on the items both judged, averaged over the raters with a κ. Returns
/// the mean and each rater's κ.
pub fn kappa_vs_raters<'a>(
    system: &BTreeMap<&'a str, bool>,
    raters: &BTreeMap<&'a str, BTreeMap<&'a str, bool>>,
) -> Option<(f64, BTreeMap<&'a str, f64>)> {
    let each: BTreeMap<&str, f64> = raters
        .iter()
        .filter_map(|(rater, labels)| {
            let pairs: Vec<(f64, f64)> = labels
                .iter()
                .filter_map(|(item, r)| {
                    system
                        .get(item)
                        .map(|y| (f64::from(u8::from(*y)), f64::from(u8::from(*r))))
                })
                .collect();
            // Both classes on both sides keep the table 2×2 in a fixed order.
            let mut t = vec![vec![0.0; 2]; 2];
            for (y, r) in pairs {
                t[1 - y as usize][1 - r as usize] += 1.0;
            }
            agreement::cohen_kappa(&t).map(|k| (*rater, k))
        })
        .collect();
    let mean = crate::numeric::mean(each.values().copied())?;
    Some((mean, each))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voters(sets: &[&[&'static str]]) -> Vec<BTreeSet<&'static str>> {
        sets.iter().map(|s| s.iter().copied().collect()).collect()
    }

    #[test]
    fn overlap_decides_nested_or_crossed() {
        // Twenty items, each by its own pair of raters: every rater has 2 items, under
        // 20% of 20.
        let names: Vec<String> = (0..40).map(|i| format!("r{i}")).collect();
        let nested: Vec<BTreeSet<&str>> = (0..20)
            .map(|i| BTreeSet::from([names[2 * i].as_str(), names[2 * i + 1].as_str()]))
            .collect();
        let d = design(nested.clone(), RaterDesign::Auto).unwrap();
        assert_eq!(
            (d.rater_design, d.detected, d.voters),
            (Design::Nested, true, 40)
        );
        assert_eq!(d.raters_per_item, BTreeMap::from([(2, 20)]));
        assert_eq!(d.estimand, "performance against one rater drawn at random");

        // Three raters on every item, one item with only two of them.
        let mut crossed = voters(&[&["ana", "ben", "cy"] as &[&str]; 19]);
        crossed.push(BTreeSet::from(["ana", "ben"]));
        let d = design(crossed, RaterDesign::Auto).unwrap();
        assert_eq!((d.rater_design, d.voters), (Design::Crossed, 3));
        assert_eq!(d.raters_per_item, BTreeMap::from([(2, 1), (3, 19)]));

        // A setting overrides the overlap; no votes, no design.
        let d = design(nested, RaterDesign::Crossed).unwrap();
        assert_eq!((d.rater_design, d.detected), (Design::Crossed, false));
        assert_eq!(design(voters(&[&[]]), RaterDesign::Auto), None);
    }
}
