//! Agreement between raters (metrics §9.1) and the variability of one rater (§9.5).
//!
//! Tables are square Q×Q counts with rows for the first rater and columns for the
//! second. `table` orders categories ascending, so for 0/1 values index 0 is the
//! negative class; `specific_agreement` instead takes `[[a, b], [c, d]]` with `a` as
//! both positive, so flip a `table` result before passing it.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::numeric::{Running, sum};
use crate::records::{Label, LabelKind, Source};

/// Ascending distinct categories and the Q×Q table of the pairs `(first, second)`.
pub fn table(pairs: &[(f64, f64)]) -> (Vec<f64>, Vec<Vec<f64>>) {
    let mut categories: Vec<f64> = pairs
        .iter()
        .flat_map(|&(a, b)| [a + 0.0, b + 0.0])
        .collect();
    categories.sort_by(f64::total_cmp);
    categories.dedup();
    // `+ 0.0` turns -0.0 into 0.0: `dedup` merges the two zeros, `total_cmp` would not.
    let at = |x: f64| categories.partition_point(|c| c.total_cmp(&(x + 0.0)).is_lt());
    let mut t = vec![vec![0.0; categories.len()]; categories.len()];
    for &(a, b) in pairs {
        t[at(a)][at(b)] += 1.0;
    }
    (categories, t)
}

/// Observed proportions with their row and column marginals, if the table is square
/// with at least two categories and a positive total.
struct Proportions {
    p: Vec<Vec<f64>>,
    rows: Vec<f64>,
    cols: Vec<f64>,
}

fn proportions(t: &[Vec<f64>]) -> Option<Proportions> {
    let q = t.len();
    if q < 2 || t.iter().any(|r| r.len() != q) {
        return None;
    }
    let n = sum(t.iter().flatten().copied());
    if n <= 0.0 {
        return None;
    }
    let p: Vec<Vec<f64>> = t
        .iter()
        .map(|r| r.iter().map(|x| x / n).collect())
        .collect();
    let rows = p.iter().map(|r| sum(r.iter().copied())).collect();
    let cols = (0..q).map(|j| sum(p.iter().map(|r| r[j]))).collect();
    Some(Proportions { p, rows, cols })
}

fn chance_corrected(po: f64, pe: f64) -> Option<f64> {
    (pe < 1.0).then(|| (po - pe) / (1.0 - pe))
}

/// Cohen's κ = (p_o − p_e)/(1 − p_e).
pub fn cohen_kappa(t: &[Vec<f64>]) -> Option<f64> {
    let s = proportions(t)?;
    let po = sum((0..t.len()).map(|i| s.p[i][i]));
    let pe = sum(s.rows.iter().zip(&s.cols).map(|(r, c)| r * c));
    chance_corrected(po, pe)
}

/// Quadratic weighted κ = 1 − Σ w·p_o / Σ w·p_e with w_ab = (a − b)²/(Q − 1)², the rows
/// and columns being the consecutive levels 0 … Q − 1.
pub fn quadratic_weighted_kappa(t: &[Vec<f64>]) -> Option<f64> {
    let levels: Vec<f64> = (0..t.len()).map(|i| i as f64).collect();
    quadratic_weighted_kappa_at(&levels, t)
}

/// The same with the level of each row and column given, ascending, so a table of the
/// levels that occur weighs them by their distance and unseen levels need no row.
pub fn quadratic_weighted_kappa_at(levels: &[f64], t: &[Vec<f64>]) -> Option<f64> {
    let s = proportions(t)?;
    let q = t.len();
    if levels.len() != q {
        return None;
    }
    let span = levels[q - 1] - levels[0];
    let w = |a: usize, b: usize| ((levels[a] - levels[b]) / span).powi(2);
    let cells = || (0..q).flat_map(|a| (0..q).map(move |b| (a, b)));
    let observed = sum(cells().map(|(a, b)| w(a, b) * s.p[a][b]));
    let expected = sum(cells().map(|(a, b)| w(a, b) * s.rows[a] * s.cols[b]));
    (expected > 0.0).then(|| 1.0 - observed / expected)
}

/// Gwet's AC1, with p_e = (1/(Q − 1)) Σ π_q(1 − π_q) and π_q the share of category q
/// over both raters.
pub fn gwet_ac1(t: &[Vec<f64>]) -> Option<f64> {
    let s = proportions(t)?;
    let q = t.len();
    let po = sum((0..q).map(|i| s.p[i][i]));
    let pe = sum(s.rows.iter().zip(&s.cols).map(|(r, c)| {
        let pi = (r + c) / 2.0;
        pi * (1.0 - pi)
    })) / (q - 1) as f64;
    chance_corrected(po, pe)
}

/// Positive agreement 2a/(2a + b + c) and negative agreement 2d/(2d + b + c) of a
/// 2×2 table; each is `None` when its denominator is 0.
pub fn specific_agreement(t: &[Vec<f64>]) -> Option<(Option<f64>, Option<f64>)> {
    if t.len() != 2 || t.iter().any(|r| r.len() != 2) {
        return None;
    }
    let (a, b, c, d) = (t[0][0], t[0][1], t[1][0], t[1][1]);
    let ratio = |x: f64| (2.0 * x + b + c > 0.0).then(|| 2.0 * x / (2.0 * x + b + c));
    Some((ratio(a), ratio(d)))
}

/// The difference function δ² of Krippendorff's α.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Nominal: 0 for equal values, 1 otherwise.
    Nominal,
    /// Ordinal: squared distance between the categories, measured through their marginal
    /// counts.
    Ordinal,
    /// Interval: squared difference of the numeric values.
    Interval,
}

/// Krippendorff's α = 1 − (n − 1)·Σ o_ck δ²_ck / Σ n_c n_k δ²_ck from the coincidence
/// matrix. Each unit lists the values its raters gave, missing ones left out; units
/// with fewer than two values are not pairable and do not count.
pub fn krippendorff_alpha(units: &[Vec<f64>], level: Level) -> Option<f64> {
    let mut categories: Vec<f64> = units
        .iter()
        .filter(|u| u.len() >= 2)
        .flatten()
        .map(|&x| x + 0.0)
        .collect();
    categories.sort_by(f64::total_cmp);
    categories.dedup();
    let q = categories.len();
    let at = |x: f64| categories.partition_point(|c| c.total_cmp(&(x + 0.0)).is_lt());
    let mut acc = vec![vec![Running::default(); q]; q];
    for u in units.iter().filter(|u| u.len() >= 2) {
        let share = 1.0 / (u.len() - 1) as f64;
        for (i, &x) in u.iter().enumerate() {
            for (j, &y) in u.iter().enumerate() {
                if i != j {
                    acc[at(x)][at(y)].add(share);
                }
            }
        }
    }
    let o: Vec<Vec<f64>> = acc
        .iter()
        .map(|r| r.iter().map(Running::total).collect())
        .collect();
    let marginals: Vec<f64> = o.iter().map(|r| sum(r.iter().copied())).collect();
    let n = sum(marginals.iter().copied());
    let delta = |c: usize, k: usize| match level {
        Level::Nominal => f64::from(u8::from(c != k)),
        Level::Ordinal => {
            let (lo, hi) = (c.min(k), c.max(k));
            let between = sum(marginals[lo..=hi].iter().copied());
            (between - (marginals[c] + marginals[k]) / 2.0).powi(2)
        }
        Level::Interval => (categories[c] - categories[k]).powi(2),
    };
    let cells = || (0..q).flat_map(|c| (0..q).map(move |k| (c, k)));
    let observed = sum(cells().map(|(c, k)| o[c][k] * delta(c, k)));
    let expected = sum(cells().map(|(c, k)| marginals[c] * marginals[k] * delta(c, k)));
    (expected > 0.0).then(|| 1.0 - (n - 1.0) * observed / expected)
}

/// Ranks from 1 with ties given their mean rank.
fn midranks(x: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..x.len()).collect();
    order.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let mut ranks = vec![0.0; x.len()];
    let mut start = 0;
    while start < order.len() {
        let end = start
            + order[start..]
                .iter()
                .take_while(|&&i| x[i] == x[order[start]])
                .count();
        let rank = (start + end + 1) as f64 / 2.0;
        for &i in &order[start..end] {
            ranks[i] = rank;
        }
        start = end;
    }
    ranks
}

/// Spearman's ρ: the Pearson correlation of the midranks.
pub fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() != y.len() || x.len() < 2 || x.iter().chain(y).any(|v| v.is_nan()) {
        return None;
    }
    let (rx, ry) = (midranks(x), midranks(y));
    let mean = (x.len() + 1) as f64 / 2.0;
    let pairs = || rx.iter().zip(&ry);
    let sxy = sum(pairs().map(|(a, b)| (a - mean) * (b - mean)));
    let sxx = sum(rx.iter().map(|a| (a - mean).powi(2)));
    let syy = sum(ry.iter().map(|b| (b - mean).powi(2)));
    (sxx > 0.0 && syy > 0.0).then(|| sxy / (sxx * syy).sqrt())
}

/// Pairs of labels on the same item, and how many of them differ.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LabelDisagreement {
    /// Items with at least two labels.
    pub items: usize,
    /// Pairs of labels compared, summed over items.
    pub pairs: usize,
    /// Pairs whose two values differ.
    pub disagreeing: usize,
}

impl LabelDisagreement {
    /// Share of compared pairs that differ; `None` without a pair.
    pub fn rate(&self) -> Option<f64> {
        (self.pairs > 0).then(|| self.disagreeing as f64 / self.pairs as f64)
    }

    fn add(&mut self, values: &[f64]) {
        if values.len() < 2 {
            return;
        }
        self.items += 1;
        for (i, a) in values.iter().enumerate() {
            for b in &values[i + 1..] {
                self.pairs += 1;
                self.disagreeing += usize::from(a != b);
            }
        }
    }
}

/// Item -> rater -> (label replicate, value), for one task.
type ByItem<'a> = BTreeMap<&'a str, BTreeMap<&'a str, Vec<(u32, f64)>>>;

/// Disagreement within each human rater (their label replicates of one item) and between
/// human raters (each one's first label replicate of one item), per task. Pass the latest
/// revision of each label; labels from other sources and adjudications, which are not
/// independent, are left out.
pub fn variability<'a>(
    labels: impl IntoIterator<Item = &'a Label>,
) -> BTreeMap<String, (LabelDisagreement, LabelDisagreement)> {
    let mut by: BTreeMap<&str, ByItem<'_>> = BTreeMap::new();
    for l in labels {
        if l.source != Source::Human || l.kind() == LabelKind::Adjudication {
            continue;
        }
        if let Some(v) = l.value {
            by.entry(l.task())
                .or_default()
                .entry(&l.item_id)
                .or_default()
                .entry(&l.rater_id)
                .or_default()
                .push((l.label_replicate(), v.as_f64()));
        }
    }
    by.into_iter()
        .map(|(task, items)| {
            let (mut within, mut between) =
                (LabelDisagreement::default(), LabelDisagreement::default());
            for raters in items.into_values() {
                let mut firsts = Vec::new();
                for mut replicates in raters.into_values() {
                    replicates.sort_by_key(|(r, _)| *r);
                    firsts.push(replicates[0].1);
                    within.add(&replicates.iter().map(|(_, v)| *v).collect::<Vec<_>>());
                }
                between.add(&firsts);
            }
            (task.to_owned(), (within, between))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::Record;

    fn label(item: &str, rater: &str, replicate: u32, kind: &str, value: bool) -> Label {
        let source = if rater.starts_with("model") {
            "model"
        } else {
            "human"
        };
        let json = format!(
            r#"{{"record":"label","item_id":"{item}","rater_id":"{rater}","source":"{source}","kind":"{kind}","label_replicate":{replicate},"value":{value},"version":1}}"#
        );
        match Record::from_json(&json).unwrap() {
            Record::Label(l) => l,
            _ => unreachable!(),
        }
    }

    #[test]
    fn variability_separates_within_and_between_human_raters() {
        let labels = [
            label("q1", "ana", 0, "independent", true),
            label("q1", "ana", 1, "independent", false),
            label("q1", "ben", 0, "independent", true),
            label("q2", "ana", 0, "independent", true),
            label("q2", "ben", 0, "independent", false),
            label("q2", "cy", 0, "adjudication", true),
            label("q1", "model-a", 0, "independent", false),
            label("q1", "model-a", 1, "independent", true),
        ];
        let (within, between) = variability(&labels)["default"];
        assert_eq!((within.items, within.pairs, within.disagreeing), (1, 1, 1));
        assert_eq!(
            (between.items, between.pairs, between.disagreeing),
            (2, 2, 1)
        );
        assert_eq!(between.rate(), Some(0.5));
    }

    #[test]
    fn negative_zero_is_the_category_zero() {
        // `-0.0` and `0.0` are one category: the table is [0, 1] with both pairs at (0, 1).
        let (categories, t) = table(&[(-0.0, 1.0), (0.0, 1.0)]);
        assert_eq!(categories, [0.0, 1.0]);
        assert_eq!(t, [[0.0, 2.0], [0.0, 0.0]]);
        assert_eq!(table(&[(-0.0, 0.0), (0.0, 0.0)]).1, [[2.0]]);
        // Krippendorff's α is the one of the +0.0 data (hand value -0.25).
        let mixed = krippendorff_alpha(
            &[vec![-0.0, 1.0], vec![0.0, 1.0], vec![1.0, 1.0]],
            Level::Nominal,
        );
        let plain = krippendorff_alpha(
            &[vec![0.0, 1.0], vec![0.0, 1.0], vec![1.0, 1.0]],
            Level::Nominal,
        );
        assert_eq!(mixed, plain);
        assert!((plain.unwrap() - -0.25).abs() < 1e-12);
    }

    #[test]
    fn table_orders_categories_and_degenerate_inputs_have_no_value() {
        let (cats, t) = table(&[(1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]);
        assert_eq!(cats, [0.0, 1.0]);
        assert_eq!(t, [vec![1.0, 0.0], vec![1.0, 1.0]]);
        assert_eq!(cohen_kappa(&[vec![3.0]]), None);
        assert_eq!(cohen_kappa(&[vec![3.0, 0.0], vec![0.0, 0.0]]), None);
        assert_eq!(
            krippendorff_alpha(&[vec![1.0], vec![2.0]], Level::Nominal),
            None
        );
        assert_eq!(spearman(&[1.0, 1.0], &[1.0, 2.0]), None);
    }

    #[test]
    fn perfect_agreement_is_one_at_every_level() {
        let units = vec![vec![1.0, 1.0], vec![2.0, 2.0, 2.0], vec![4.0, 4.0]];
        for level in [Level::Nominal, Level::Ordinal, Level::Interval] {
            assert_eq!(krippendorff_alpha(&units, level), Some(1.0));
        }
    }
}
