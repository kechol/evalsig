//! Agent-side dev / validation splits (SPEC §8.1, D-022). Clusters are the unit, so
//! items of one document never land on both sides.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::numeric::Rng;
use crate::records::Item;
use crate::time::Instant;

/// How items are assigned.
#[derive(Debug, Clone, PartialEq)]
pub enum Method {
    /// The latest clusters (by their earliest `occurred_at`) go to validation.
    Time,
    /// Clusters shuffled by the seed, within each value of `stratum` when given.
    Random { seed: u64, stratum: Option<String> },
    /// The user's assignment of each item.
    Assign(BTreeMap<String, Part>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    Dev,
    Validation,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SplitError {
    #[error("cluster {cluster:?} has items assigned to both dev and validation")]
    CrossesCluster { cluster: String },
    #[error("item {0:?} is not among the items the split may use (sealed holdout or unknown)")]
    NotAvailable(String),
    #[error("the assignment leaves item {0:?} out")]
    Unassigned(String),
    #[error("item {0:?} has no occurred_at, so a time split cannot place it")]
    Undated(String),
    #[error("{0}")]
    Timestamp(String),
    #[error("the validation share must lie strictly between 0 and 1")]
    Share,
    #[error("there are no items to split")]
    Empty,
}

/// Item IDs of each part, sorted.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Assignment {
    pub dev: Vec<String>,
    pub validation: Vec<String>,
}

/// Split `items` (restricted to `within` when given) so that about `share` of the
/// items, counted by whole clusters, form validation.
pub fn split(
    items: &[&Item],
    within: Option<&BTreeSet<String>>,
    method: &Method,
    share: f64,
) -> Result<Assignment, SplitError> {
    if !(share > 0.0 && share < 1.0) {
        return Err(SplitError::Share);
    }
    if let (Method::Assign(map), Some(allowed)) = (method, within)
        && let Some(id) = map.keys().find(|id| !allowed.contains(*id))
    {
        return Err(SplitError::NotAvailable(id.clone()));
    }
    let usable: Vec<&Item> = items
        .iter()
        .copied()
        .filter(|i| within.is_none_or(|w| w.contains(&i.item_id)))
        .collect();
    if usable.is_empty() {
        return Err(SplitError::Empty);
    }
    let mut clusters: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for i in &usable {
        clusters.entry(i.cluster_key()).or_default().push(i);
    }
    let parts: BTreeMap<&str, Part> = match method {
        Method::Assign(map) => {
            let mut out = BTreeMap::new();
            for (cluster, members) in &clusters {
                let mut sides = BTreeSet::new();
                for i in members {
                    let side = map
                        .get(&i.item_id)
                        .ok_or_else(|| SplitError::Unassigned(i.item_id.clone()))?;
                    sides.insert(*side);
                }
                if sides.len() > 1 {
                    return Err(SplitError::CrossesCluster {
                        cluster: (*cluster).to_owned(),
                    });
                }
                out.insert(*cluster, *sides.first().expect("a cluster has an item"));
            }
            out
        }
        Method::Time => {
            let mut order: Vec<(Instant, &str, usize)> = Vec::new();
            for (cluster, members) in &clusters {
                let mut first: Option<Instant> = None;
                for i in members {
                    let at = i
                        .occurred_at
                        .as_deref()
                        .ok_or_else(|| SplitError::Undated(i.item_id.clone()))?;
                    let at =
                        Instant::parse(at).map_err(|e| SplitError::Timestamp(e.to_string()))?;
                    first = Some(first.map_or(at, |f| f.min(at)));
                }
                order.push((
                    first.expect("a cluster has an item"),
                    cluster,
                    members.len(),
                ));
            }
            order.sort();
            latest_share(order.into_iter().map(|(_, c, n)| (c, n)).collect(), share)
        }
        Method::Random { seed, stratum } => {
            let mut groups: BTreeMap<String, Vec<(&str, usize)>> = BTreeMap::new();
            for (cluster, members) in &clusters {
                let key = stratum
                    .as_ref()
                    .and_then(|s| members[0].strata.as_ref()?.get(s))
                    .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                    .unwrap_or_default();
                groups
                    .entry(key)
                    .or_default()
                    .push((cluster, members.len()));
            }
            let mut rng = Rng::new(*seed);
            let mut out = BTreeMap::new();
            for mut g in groups.into_values() {
                for i in (1..g.len()).rev() {
                    g.swap(i, rng.below(i as u64 + 1) as usize);
                }
                out.extend(latest_share(g, share));
            }
            out
        }
    };
    let mut a = Assignment::default();
    for (cluster, members) in &clusters {
        let ids = members.iter().map(|i| i.item_id.clone());
        match parts[cluster] {
            Part::Dev => a.dev.extend(ids),
            Part::Validation => a.validation.extend(ids),
        }
    }
    a.dev.sort();
    a.validation.sort();
    Ok(a)
}

/// Validation takes clusters from the end of `ordered` until it holds `share` of the
/// items; the rest is dev. Each side keeps at least one cluster when there are two.
fn latest_share(ordered: Vec<(&str, usize)>, share: f64) -> BTreeMap<&str, Part> {
    let total: usize = ordered.iter().map(|(_, n)| n).sum();
    let target = share * total as f64;
    let mut out = BTreeMap::new();
    let mut taken = 0;
    let last = ordered.len();
    for (k, (cluster, n)) in ordered.into_iter().enumerate().rev() {
        let keep_dev = k == 0 && last > 1;
        let validation = !keep_dev && ((taken as f64) < target || k + 1 == last);
        if validation {
            taken += n;
        }
        out.insert(
            cluster,
            if validation {
                Part::Validation
            } else {
                Part::Dev
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::Record;

    fn items(lines: &[String]) -> Vec<Item> {
        lines
            .iter()
            .map(|l| match Record::from_json(l).unwrap() {
                Record::Item(i) => i,
                _ => unreachable!(),
            })
            .collect()
    }

    /// Twenty items in ten clusters of two, one cluster per day.
    fn sample() -> Vec<Item> {
        items(
            &(0..20)
                .map(|i| {
                    format!(
                        r#"{{"record":"item","item_id":"i{i:02}","cluster":"c{}","occurred_at":"2026-10-{:02}T00:00:00Z","strata":{{"lang":"{}"}}}}"#,
                        i / 2,
                        i / 2 + 1,
                        if i < 10 { "en" } else { "ja" }
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    fn no_cluster_crosses(items: &[Item], a: &Assignment) {
        let side: BTreeMap<&str, bool> = a
            .dev
            .iter()
            .map(|i| (i.as_str(), false))
            .chain(a.validation.iter().map(|i| (i.as_str(), true)))
            .collect();
        let mut by_cluster: BTreeMap<&str, BTreeSet<bool>> = BTreeMap::new();
        for i in items {
            by_cluster
                .entry(i.cluster_key())
                .or_default()
                .insert(side[i.item_id.as_str()]);
        }
        assert!(by_cluster.values().all(|s| s.len() == 1));
    }

    #[test]
    fn time_splits_put_the_latest_clusters_in_validation() {
        let s = sample();
        let refs: Vec<&Item> = s.iter().collect();
        let a = split(&refs, None, &Method::Time, 0.3).unwrap();
        assert_eq!(a.validation, ["i14", "i15", "i16", "i17", "i18", "i19"]);
        assert_eq!(a.dev.len(), 14);
        no_cluster_crosses(&s, &a);
    }

    #[test]
    fn random_splits_keep_clusters_whole_within_each_stratum() {
        let s = sample();
        let refs: Vec<&Item> = s.iter().collect();
        let method = Method::Random {
            seed: 7,
            stratum: Some("lang".into()),
        };
        let a = split(&refs, None, &method, 0.4).unwrap();
        no_cluster_crosses(&s, &a);
        // Two of five clusters in each language.
        let ja = a.validation.iter().filter(|i| i.as_str() >= "i10").count();
        assert_eq!((a.validation.len(), ja), (8, 4));
        assert_eq!(split(&refs, None, &method, 0.4).unwrap(), a);
    }

    #[test]
    fn assignments_that_cross_a_cluster_or_use_withheld_items_are_refused() {
        let s = sample();
        let refs: Vec<&Item> = s.iter().collect();
        let mut map: BTreeMap<String, Part> =
            s.iter().map(|i| (i.item_id.clone(), Part::Dev)).collect();
        map.insert("i01".into(), Part::Validation);
        assert_eq!(
            split(&refs, None, &Method::Assign(map.clone()), 0.5),
            Err(SplitError::CrossesCluster {
                cluster: "c0".into()
            })
        );
        map.insert("i00".into(), Part::Validation);
        let ok = split(&refs, None, &Method::Assign(map.clone()), 0.5).unwrap();
        assert_eq!(ok.validation, ["i00", "i01"]);
        // The judge kept i19 (cluster c9) back for the holdout.
        let within: BTreeSet<String> = s
            .iter()
            .map(|i| i.item_id.clone())
            .filter(|i| i != "i18" && i != "i19")
            .collect();
        assert_eq!(
            split(&refs, Some(&within), &Method::Assign(map), 0.5),
            Err(SplitError::NotAvailable("i18".into()))
        );
        let t = split(&refs, Some(&within), &Method::Time, 0.3).unwrap();
        assert!(!t.dev.contains(&"i19".to_owned()) && !t.validation.contains(&"i19".to_owned()));
    }
}
