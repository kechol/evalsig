//! Metrics over units of several items (D-051, D-053): the items of a cluster taken as
//! one unit, which is positive or flagged when any of its items is.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{Fill, Metric, MetricId, Prepared, ReportError, Settings, Unit, Values};
use crate::config::Estimand;
use crate::missing::Side;
use crate::numeric::sum;
use crate::records::{Item, ValueSemantics};

/// How the items of a unit combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    /// The largest of the items' reference shares and judged shares.
    Any,
}

/// Accuracy, hit rate and false-alarm rate over clusters taken as units.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnitMetrics {
    /// How a unit's items combine.
    pub aggregate: Aggregate,
    /// Units whose items all have a judgment and a reference.
    pub n_units: usize,
    /// Units with an item without a judgment: none in the run, or none left under the
    /// missing policy.
    pub n_missing: usize,
    /// Units whose items all have a judgment, one of them without a reference.
    pub n_without_reference: usize,
    /// The metrics over units, each unit independent.
    pub metrics: Vec<Metric>,
}

/// A cluster whose items all have a judgment and a reference.
struct Group<'u> {
    key: &'u str,
    members: Vec<&'u Unit>,
    reference: f64,
}

impl Prepared<'_> {
    /// The metrics over units, a cluster's items being those of `items` with its key.
    pub(super) fn units(
        &self,
        items: &BTreeMap<&str, &Item>,
        aggregate: Aggregate,
    ) -> Result<UnitMetrics, ReportError> {
        if self.semantics != ValueSemantics::Decision {
            return Err(ReportError::UnitsNeedDecisions);
        }
        if self.units.iter().any(|u| u.inclusion_weighted) {
            return Err(ReportError::UnitsWithInclusionWeights);
        }
        let mut size: BTreeMap<&str, usize> = BTreeMap::new();
        for item in items.values() {
            *size.entry(item.cluster_key()).or_default() += 1;
        }
        let mut clusters: BTreeMap<&str, Vec<&Unit>> = BTreeMap::new();
        for u in &self.units {
            clusters.entry(u.cluster.as_str()).or_default().push(u);
        }
        // A unit's value is already combined, so it is read as one shot.
        let settings = Settings {
            estimand: Estimand::SingleShot,
            ..self.settings.clone()
        };
        let item_calc = self.calc();
        let calc = super::Calc {
            settings: &settings,
            ..item_calc
        };
        let (mut groups, mut n_missing, mut n_without_reference) = (Vec::new(), 0, 0);
        for (key, members) in clusters {
            // Whether a value is left does not depend on the fill, only on the policy.
            let judged = members.len() >= size.get(key).copied().unwrap_or(0)
                && members
                    .iter()
                    .all(|u| item_calc.item_value(u, |_| None).is_some());
            if !judged {
                n_missing += 1;
            } else if let Some(reference) = max(members.iter().map(|u| u.reference)) {
                groups.push(Group {
                    key,
                    members,
                    reference,
                });
            } else {
                n_without_reference += 1;
            }
        }
        let system_gaps = groups.iter().flat_map(|g| &g.members).any(|u| {
            u.replicates
                .iter()
                .any(|r| r.is_err_and(|m| m.side() == Side::System))
        });
        let metrics = [
            MetricId::Accuracy,
            MetricId::HitRate,
            MetricId::FalseAlarmRate,
        ]
        .into_iter()
        .map(|id| {
            let d = calc.definition(id);
            // Each item is filled as the metric fills its unit: under `any`, the unit's
            // worst value is reached by filling every item with it.
            let units = |bound: Fill| -> Vec<Unit> {
                groups
                    .iter()
                    .map(|g| {
                        let mut unit = Unit {
                            item_id: g.key.to_owned(),
                            cluster: g.key.to_owned(),
                            weight: sum(g.members.iter().map(|u| u.weight))
                                / g.members.len() as f64,
                            reference: Some(g.reference),
                            ..Unit::default()
                        };
                        let worst = (d.worst)(&unit);
                        let value = match aggregate {
                            Aggregate::Any => max(g.members.iter().map(|u| {
                                item_calc.item_value_bound(u, &|_| worst, bound, Values::Judged)
                            })),
                        };
                        unit.replicates = vec![Ok(value.expect("judged units have a value"))];
                        unit
                    })
                    .collect()
            };
            let point = units(Fill::Policy);
            let all: Vec<&Unit> = point.iter().collect();
            let mut m = calc.ratio_of(id.name(), &all, &*d.worst, &*d.part, d.values);
            let total = |bound: Fill| {
                let parts: Vec<(f64, f64)> = units(bound)
                    .iter()
                    .map(|u| (d.part)(u, u.replicates[0].expect("a value")))
                    .collect();
                let den = sum(parts.iter().map(|p| p.1));
                (den > 0.0).then(|| sum(parts.iter().map(|p| p.0)) / den)
            };
            if system_gaps && let (Some(a), Some(b)) = (total(Fill::Worst), total(Fill::Best)) {
                m.missing_bounds = Some((a.min(b), a.max(b)));
            }
            m
        })
        .collect();
        Ok(UnitMetrics {
            aggregate,
            n_units: groups.len(),
            n_missing,
            n_without_reference,
            metrics,
        })
    }
}

/// The largest value, or `None` when any is absent.
fn max(mut values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    values.try_fold(f64::NEG_INFINITY, |m, v| Some(m.max(v?)))
}
