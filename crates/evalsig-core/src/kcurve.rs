//! Performance and cost as the number of production generations k varies (metrics
//! spec §6.1.1, `report --k-curve`): for k = 1 … min K_i, the metrics under
//! `majority@k`, `any@k` and `all@k`, and the expected cost per item.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::config::Estimand;
use crate::numeric::Running;
use crate::records::{Item, Judgment, Label, Run};
use crate::report::{self, Metric, ReportError, Sections, Settings};

/// The metrics of one k-run rule at one k.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Point {
    /// The rule that combines k generations: `majority`, `any` or `all`.
    pub rule: &'static str,
    /// Number of generation replicates combined.
    pub k: u32,
    /// The run's metrics under the rule at this k.
    pub metrics: Vec<Metric>,
    /// k times the mean cost of one judgment, per cost key.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub cost_per_item: BTreeMap<String, f64>,
}

/// Performance as a function of the number of generation replicates k.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KCurve {
    /// The smallest number of generation replicates of any item (min K_i).
    pub k_max: u32,
    /// One point per rule and k, for k from 1 to `k_max`.
    pub points: Vec<Point>,
}

/// The k-curve of a run's task.
pub fn build(
    run: &Run,
    judgments: &[&Judgment],
    items: &BTreeMap<&str, &Item>,
    labels: &[&Label],
    s: &Settings,
) -> Result<KCurve, ReportError> {
    let task: Vec<&Judgment> = judgments
        .iter()
        .copied()
        .filter(|j| j.task() == s.task)
        .collect();
    let mut replicates: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    for j in &task {
        replicates
            .entry(j.item_id.as_str())
            .or_default()
            .insert(j.gen_replicate);
    }
    let k_max = replicates
        .values()
        .map(|r| r.len() as u32)
        .min()
        .ok_or_else(|| ReportError::NoJudgments(run.run_id.clone(), s.task.clone()))?;
    // Mean cost of one judgment per key, over the judgments that report it.
    let mut sums: BTreeMap<String, (Running, u32)> = BTreeMap::new();
    for j in &task {
        for (k, v) in j.cost.iter().flatten() {
            if let Some(x) = v.as_f64() {
                let e = sums.entry(k.clone()).or_default();
                e.0.add(x);
                e.1 += 1;
            }
        }
    }
    let mut points = Vec::new();
    for k in 1..=k_max {
        let rules = [
            ("majority", Estimand::Majority(k)),
            ("any", Estimand::Any(k)),
            ("all", Estimand::All(k)),
        ];
        for (rule, estimand) in rules {
            let settings = Settings {
                estimand,
                // Each point is read for its metrics only.
                sections: Sections::default(),
                proxy: None,
                ..s.clone()
            };
            let r = report::build(run, judgments, items, labels, &settings)?;
            points.push(Point {
                rule,
                k,
                metrics: r.metrics,
                cost_per_item: sums
                    .iter()
                    .map(|(key, (sum, n))| {
                        (key.clone(), f64::from(k) * sum.total() / f64::from(*n))
                    })
                    .collect(),
            });
        }
    }
    Ok(KCurve { k_max, points })
}
