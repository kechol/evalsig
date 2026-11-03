//! Building one reference from several raters' labels (metrics spec §9.2).
//!
//! Under `soft` every metric is the value against one rater drawn at random: an item
//! with positive share s_i counts s_i toward the positive side and 1 − s_i toward the
//! negative side. `single` and `majority` give shares of 0, ½ (a tie) or 1.

use std::collections::BTreeMap;

use crate::agreement;
use crate::config::ReferenceMode;
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

/// The reference of one item from the latest revision of its labels. Human labels
/// are used when there are any; adjudications decide under `single` and are left out
/// of the votes otherwise, as are later label replicates of the same rater.
pub fn of(labels: &[&Label], positive: &Rule, mode: ReferenceMode) -> Option<Reference> {
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
    let value = |l: &Label| l.value.and_then(|v| positive.is_positive(v));
    if mode == ReferenceMode::Single
        && let Some(adj) = pool.iter().find(|l| l.kind() == LabelKind::Adjudication)
    {
        return value(adj).map(|p| Reference {
            share: f64::from(u8::from(p)),
            model_derived,
        });
    }
    let votes: Vec<bool> = pool
        .iter()
        .filter(|l| l.kind() != LabelKind::Adjudication && l.label_replicate() == 0)
        .filter_map(|l| value(l))
        .collect();
    if votes.is_empty() {
        return None;
    }
    let share = votes.iter().filter(|v| **v).count() as f64 / votes.len() as f64;
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
