//! The detail tier of a report (SPEC §6): ROC and AUC (metrics spec §3), calibration
//! (§3.4) and feature likelihood ratios (§5), from the units `prepare` built.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{Fill, Prepared, ReportError, Unit};
use crate::cost::{CostPart, Sums};
use crate::features::{self, Class, Counts, Thresholds};
use crate::numeric::{Running, mean, sum};
use crate::ppi;
use crate::records::{Item, Label, Source};
use crate::records::{OutputType, ValueSemantics};
use crate::reference;
use crate::roc::{self, Bin, SoftItem, Weighted};

/// The empirical ROC of a run's scores or levels against the reference.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Roc {
    /// Area under the ROC curve.
    pub auc: f64,
    /// Interval of the AUC; absent when none can be computed.
    pub ci: Option<(f64, f64)>,
    /// `delong`, `obuchowski` (clusters), `bootstrap` (soft or weighted references,
    /// or an AUC of 0 or 1) or `too_few_units`.
    pub interval: &'static str,
    /// Weighted number of positives.
    pub n_positive: f64,
    /// Weighted number of negatives.
    pub n_negative: f64,
    /// (false-alarm rate, hit rate) from the strongest threshold.
    pub points: Vec<(f64, f64)>,
    /// (z(F′), z(H′)) per threshold between levels, for ordinal runs with a 0/1
    /// reference; a slope far from 1 argues against the equal-variance d′.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zroc: Option<Vec<(f64, f64)>>,
}

/// Sensitivity d′ and criterion c of the decision rule (metrics spec §2), from the
/// same weighted counts as H and F, with the log-linear correction on every cell.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sdt {
    /// d′ = z(H′) − z(F′).
    pub dprime: f64,
    /// c = −(z(H′) + z(F′)) / 2; negative means liberal.
    pub criterion: f64,
    /// One ROC point gives d′ and c only under this assumption.
    pub assumption: &'static str,
}

/// Calibration of scores read as probabilities.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Calibration {
    /// Brier score.
    pub brier: f64,
    /// Reliability diagram bins.
    pub bins: Vec<Bin>,
}

/// Likelihood ratio of one feature.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeatureRow {
    /// Feature ID.
    pub feature_id: String,
    /// Weighted firings among positives and negatives, and the class sizes.
    pub fired_positive: f64,
    /// Weighted number of positives.
    pub n_positive: f64,
    /// Weighted firings among negatives.
    pub fired_negative: f64,
    /// Weighted number of negatives.
    pub n_negative: f64,
    /// Likelihood ratio; 1 means uninformative.
    pub lr: f64,
    /// Interval of the likelihood ratio at the report level.
    pub ci: (f64, f64),
    /// The interval at the Benjamini-Hochberg adjusted level, which decides the class.
    pub adjusted_ci: (f64, f64),
    /// Two-sided p-value of the hypothesis that the likelihood ratio is 1.
    pub p_value: f64,
    /// True when a zero count made the estimate add ½ to each cell (Haldane-Anscombe).
    pub corrected: bool,
    /// Class of the feature by its adjusted interval.
    pub class: Class,
    /// Posterior probability of a positive given the feature, at the stated prevalence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub posterior: Option<f64>,
}

impl Prepared<'_> {
    /// Units with a reference and a raw value, for decision runs whose judgments are
    /// levels or scores.
    fn ranked(&self) -> Option<Vec<&Unit>> {
        let graded = matches!(self.output_type, OutputType::Ordinal | OutputType::Score);
        (self.semantics == ValueSemantics::Decision && graded && self.positive.is_some()).then(
            || {
                self.units
                    .iter()
                    .filter(|u| u.reference.is_some() && u.raw.is_some())
                    .collect()
            },
        )
    }

    /// The ROC and AUC with its interval: DeLong for 0/1 references of single items,
    /// Obuchowski's clustered variance when items share clusters, otherwise (soft or
    /// weighted references, or an AUC of 0 or 1) a cluster bootstrap.
    pub fn roc(&self) -> Option<Roc> {
        let units = self.ranked()?;
        let s = &self.settings;
        let share = |u: &Unit| u.reference.unwrap_or(0.0);
        let score = |u: &Unit| u.raw.unwrap_or(0.0);
        let pos: Vec<Weighted> = units
            .iter()
            .map(|u| Weighted {
                score: score(u),
                weight: u.weight * share(u),
            })
            .filter(|w| w.weight > 0.0)
            .collect();
        let neg: Vec<Weighted> = units
            .iter()
            .map(|u| Weighted {
                score: score(u),
                weight: u.weight * (1.0 - share(u)),
            })
            .filter(|w| w.weight > 0.0)
            .collect();
        let auc = roc::auc(&pos, &neg)?;
        let hard = units
            .iter()
            .all(|u| u.weight == 1.0 && (share(u) == 0.0 || share(u) == 1.0));
        let cluster_ids: BTreeMap<&str, usize> = units
            .iter()
            .map(|u| u.cluster.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(i, c)| (c, i))
            .collect();
        let clustered = units.iter().any(|u| u.cluster != u.item_id);
        let analytic = hard.then(|| {
            let side = |positive: bool| -> Vec<(f64, usize)> {
                units
                    .iter()
                    .filter(|u| (share(u) == 1.0) == positive)
                    .map(|u| (score(u), cluster_ids[u.cluster.as_str()]))
                    .collect()
            };
            let (p, n) = (side(true), side(false));
            if clustered {
                roc::delong_clustered(&p, &n, s.level).map(|d| (d.interval, "obuchowski"))
            } else {
                let scores = |v: &[(f64, usize)]| v.iter().map(|x| x.0).collect::<Vec<_>>();
                roc::delong(&scores(&p), &scores(&n), s.level).map(|d| (d.interval, "delong"))
            }
        });
        let (ci, interval) = match analytic.flatten() {
            Some((Some(ci), method)) => (Some(ci), method),
            _ => {
                let items: Vec<SoftItem> = units
                    .iter()
                    .map(|u| SoftItem {
                        score: score(u),
                        share: share(u),
                        weight: u.weight,
                    })
                    .collect();
                let clusters: Vec<usize> = units
                    .iter()
                    .map(|u| cluster_ids[u.cluster.as_str()])
                    .collect();
                match roc::auc_soft_interval(&items, &clusters, s.bootstrap, s.seed, s.level) {
                    Some(ci) => (Some(ci), "bootstrap"),
                    None => (None, "too_few_units"),
                }
            }
        };
        let zroc = (hard && self.output_type == OutputType::Ordinal).then(|| {
            let mut levels: Vec<f64> = units.iter().map(|u| score(u)).collect();
            levels.sort_by(|a, b| b.total_cmp(a));
            levels.dedup();
            let hist = |positive: bool| -> Vec<u64> {
                levels
                    .iter()
                    .map(|l| {
                        units
                            .iter()
                            .filter(|u| (share(u) == 1.0) == positive && score(u) == *l)
                            .count() as u64
                    })
                    .collect()
            };
            roc::zroc_points(&hist(true), &hist(false))
        });
        Some(Roc {
            auc,
            ci,
            interval,
            n_positive: sum(pos.iter().map(|w| w.weight)),
            n_negative: sum(neg.iter().map(|w| w.weight)),
            points: roc::roc_points(&pos, &neg),
            zroc,
        })
    }

    /// Each item's part of expected cost under `[cost]` (D-041): the items with a
    /// reference and a judged level. `None` without `[cost]` or a reference, or when a
    /// judged value is not a level 0 … k − 1 of the matrix.
    pub fn cost_parts(&self) -> Option<BTreeMap<String, CostPart>> {
        let cost = self.settings.cost.as_ref()?;
        if self.semantics != ValueSemantics::Decision || self.positive.is_none() {
            return None;
        }
        let k = cost.matrix.first()?.len() as f64;
        let mut parts = BTreeMap::new();
        for u in &self.units {
            let Some(s) = u.reference else { continue };
            if u.levels.is_empty() {
                continue;
            }
            if u.levels
                .iter()
                .any(|l| l.fract() != 0.0 || *l < 0.0 || *l >= k)
            {
                return None;
            }
            let matrix = cost.matrix_of(u.cost_stratum.as_deref());
            let n = u.levels.len() as f64;
            let row = |c: usize| sum(u.levels.iter().map(|l| matrix[c][*l as usize])) / n;
            let weight = [u.weight * (1.0 - s), u.weight * s];
            parts.insert(
                u.item_id.clone(),
                CostPart {
                    cluster: u.cluster.clone(),
                    stratum: u.cost_stratum.clone(),
                    sums: Sums {
                        weight,
                        cost: [weight[0] * row(0), weight[1] * row(1)],
                    },
                },
            );
        }
        (!parts.is_empty()).then_some(parts)
    }

    /// Weighted counts of positives and negatives per judgment level 0 … levels − 1
    /// (a soft reference splits an item), for the cost of each threshold (metrics spec
    /// §4). `None` unless every ranked item's mean level is a level in range.
    pub fn level_counts(&self, levels: usize) -> Option<(Vec<f64>, Vec<f64>)> {
        let units = self.ranked()?;
        let mut pos = vec![Running::default(); levels];
        let mut neg = vec![Running::default(); levels];
        for u in &units {
            let l = u.raw?;
            if l.fract() != 0.0 || l < 0.0 || l as usize >= levels {
                return None;
            }
            let s = u.reference.unwrap_or(0.0);
            pos[l as usize].add(u.weight * s);
            neg[l as usize].add(u.weight * (1.0 - s));
        }
        let totals = |r: Vec<Running>| r.iter().map(Running::total).collect();
        (!units.is_empty()).then(|| (totals(pos), totals(neg)))
    }

    /// The weighted (hits, positives) and (false alarms, negatives) under the decision
    /// rule.
    fn counts(&self) -> Option<((f64, f64), (f64, f64))> {
        let calc = self.calc();
        let all: Vec<&Unit> = self.units.iter().collect();
        let count = |id| {
            let parts = calc.parts(id, &calc.eligible(id, &all), Fill::Policy);
            let num = sum(parts.iter().flatten().map(|p| p.0));
            let den = sum(parts.iter().flatten().map(|p| p.1));
            (den > 0.0).then_some((num, den))
        };
        Some((
            count(super::MetricId::HitRate)?,
            count(super::MetricId::FalseAlarmRate)?,
        ))
    }

    /// For the positives and for the negatives, the factor that brings the item weights of
    /// that class to sum to Kish's effective number of items, (Σw)²/Σw². Counts built from
    /// the scaled weights can be read as numbers of items, as the log-linear correction
    /// and the Katz standard error do, whatever the scale of the weights.
    fn class_scales(&self) -> (f64, f64) {
        let kish = |positive: bool| {
            let [s1, s2] = self
                .units
                .iter()
                .filter(|u| {
                    u.reference
                        .is_some_and(|s| if positive { s > 0.0 } else { s < 1.0 })
                })
                .fold([Running::default(); 2], |[mut a, mut b], u| {
                    a.add(u.weight);
                    b.add(u.weight * u.weight);
                    [a, b]
                });
            let (s1, s2) = (s1.total(), s2.total());
            if s2 > 0.0 { s1 / s2 } else { 1.0 }
        };
        (kish(true), kish(false))
    }

    /// The point hit rate and false-alarm rate under the decision rule.
    pub fn rates(&self) -> Option<(f64, f64)> {
        let ((h, p), (f, n)) = self.counts()?;
        Some((h / p, f / n))
    }

    /// d′ and c under the decision rule.
    pub fn sdt(&self) -> Option<Sdt> {
        let ((h, p), (f, n)) = self.counts()?;
        let (kp, kn) = self.class_scales();
        let (dprime, criterion) = crate::sdt::Confusion {
            hits: h * kp,
            misses: (p - h) * kp,
            false_alarms: f * kn,
            correct_rejections: (n - f) * kn,
        }
        .dprime_and_criterion();
        Some(Sdt {
            dprime,
            criterion,
            assumption: crate::sdt::ASSUMPTION,
        })
    }

    /// The Brier score and reliability bins of score runs whose scores lie in [0, 1].
    pub fn calibration(&self) -> Option<Calibration> {
        if self.output_type != OutputType::Score {
            return None;
        }
        let units = self.ranked()?;
        let scores: Vec<f64> = units.iter().map(|u| u.raw.unwrap_or(0.0)).collect();
        let y: Vec<f64> = units.iter().map(|u| u.reference.unwrap_or(0.0)).collect();
        let weights: Vec<f64> = units.iter().map(|u| u.weight).collect();
        roc::calibration(&scores, &y, &weights).map(|(brier, bins)| Calibration { brier, bins })
    }

    /// Each feature's likelihood ratio and class over the items with a reference; an
    /// item that does not report a feature counts as not firing it. `prevalence` adds
    /// the posterior probability.
    pub fn features(&self, thresholds: Thresholds, prevalence: Option<f64>) -> Vec<FeatureRow> {
        let units: Vec<&Unit> = self
            .units
            .iter()
            .filter(|u| u.reference.is_some())
            .collect();
        let ids: BTreeSet<&str> = units
            .iter()
            .flat_map(|u| u.fired.keys().map(String::as_str))
            .collect();
        let (kp, kn) = self.class_scales();
        let counts: Vec<Counts> = ids
            .iter()
            .map(|id| {
                let [mut a, mut n_pos, mut b, mut n_neg] = [Running::default(); 4];
                for u in &units {
                    let s = u.reference.unwrap_or(0.0);
                    let f = u.fired.get(*id).copied().unwrap_or(0.0);
                    a.add(u.weight * s * f);
                    n_pos.add(u.weight * s);
                    b.add(u.weight * (1.0 - s) * f);
                    n_neg.add(u.weight * (1.0 - s));
                }
                Counts {
                    a: kp * a.total(),
                    n_pos: kp * n_pos.total(),
                    b: kn * b.total(),
                    n_neg: kn * n_neg.total(),
                }
            })
            .collect();
        let assessed = features::assess(&counts, self.settings.level, thresholds);
        ids.into_iter()
            .zip(counts)
            .zip(assessed)
            .filter_map(|((id, c), a)| {
                let a = a?;
                Some(FeatureRow {
                    feature_id: id.to_owned(),
                    fired_positive: c.a,
                    n_positive: c.n_pos,
                    fired_negative: c.b,
                    n_negative: c.n_neg,
                    lr: a.ratio.lr,
                    ci: a.ratio.interval,
                    adjusted_ci: a.adjusted_interval,
                    p_value: a.p_value,
                    corrected: a.ratio.corrected,
                    class: a.class,
                    posterior: prevalence.map(|p| features::posterior(a.ratio.lr, p)),
                })
            })
            .collect()
    }
}

/// The human-scale value of a run corrected with a model rater's labels on the items
/// humans did not label (PPI++, metrics spec §9.8).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProxyEstimate {
    /// `accuracy` for decision runs, `mean_label` (the human labels' mean) otherwise.
    pub metric: &'static str,
    /// Model rater whose labels fill in.
    pub proxy: String,
    /// Corrected estimate.
    pub estimate: f64,
    /// Interval of the estimate.
    pub ci: (f64, f64),
    /// Standard error of the estimate.
    pub se: f64,
    /// Weight on the proxy correction; 0 uses the human labels alone.
    pub lambda: f64,
    /// Items with a human label (and the proxy's), and items with the proxy's only.
    pub labelled: usize,
    /// Items with the proxy's label only.
    pub unlabelled: usize,
    /// False when the human-labelled items are not known to be a random sample, so
    /// the estimate uses the human labels alone (λ = 0).
    pub corrected: bool,
}

impl Prepared<'_> {
    /// PPI++ with `proxy`'s labels as f and the human reference as y. The correction
    /// needs the human-labelled items to be a random sample, which evalsig takes as
    /// every item stating the same `label_inclusion_prob`; otherwise λ = 0.
    pub fn proxy_estimate(
        &self,
        items: &BTreeMap<&str, &Item>,
        labels: &[&Label],
        proxy: &str,
    ) -> Result<ProxyEstimate, ReportError> {
        let calc = self.calc();
        let mut by_item: BTreeMap<&str, Vec<&Label>> = BTreeMap::new();
        for l in labels.iter().filter(|l| l.task() == self.settings.task) {
            by_item.entry(l.item_id.as_str()).or_default().push(l);
        }
        // A label set's value on the item: the positive share for decisions, the mean
        // label value otherwise.
        let value = |ls: Vec<&Label>| -> Option<f64> {
            match &self.positive {
                Some(p) => reference::of(&ls, p, self.settings.reference).map(|r| r.share),
                None => mean(ls.iter().filter_map(|l| l.value.map(|v| v.as_f64()))),
            }
        };
        let (mut labelled, mut unlabelled) = (Vec::new(), Vec::new());
        for u in &self.units {
            let Some(ls) = by_item.get(u.item_id.as_str()) else {
                continue;
            };
            let human = value(
                ls.iter()
                    .copied()
                    .filter(|l| l.source == Source::Human)
                    .collect(),
            );
            let model = value(ls.iter().copied().filter(|l| l.rater_id == proxy).collect());
            let Some(g) = model else { continue };
            let score = |r: f64| match self.positive {
                Some(_) => calc
                    .item_value_bound(
                        u,
                        &|_| Some(if r >= 0.5 { 0.0 } else { 1.0 }),
                        Fill::Policy,
                        super::Values::Judged,
                    )
                    .map(|v| v * r + (1.0 - v) * (1.0 - r)),
                None => Some(r),
            };
            let Some(f) = score(g) else { continue };
            match human.and_then(score) {
                Some(y) => labelled.push((y, f)),
                None => unlabelled.push(f),
            }
        }
        let probabilities: BTreeSet<u64> = self
            .units
            .iter()
            .map(|u| {
                items
                    .get(u.item_id.as_str())
                    .and_then(|i| i.label_inclusion_prob)
                    .map_or(u64::MAX, f64::to_bits)
            })
            .collect();
        let random = probabilities.len() == 1 && !probabilities.contains(&u64::MAX);
        let fit = ppi::mean_estimate(
            &labelled,
            &unlabelled,
            self.settings.level,
            (!random).then_some(0.0),
        )
        .ok_or_else(|| ReportError::NoProxyLabels(proxy.to_owned()))?;
        Ok(ProxyEstimate {
            metric: if self.positive.is_some() {
                "accuracy"
            } else {
                "mean_label"
            },
            proxy: proxy.to_owned(),
            estimate: fit.estimate,
            ci: fit.interval,
            se: fit.se,
            lambda: fit.lambda,
            labelled: fit.labelled,
            unlabelled: fit.unlabelled,
            corrected: random,
        })
    }
}
