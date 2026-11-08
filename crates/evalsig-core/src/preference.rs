//! Pairwise preference (metrics spec §10): win rate, position bias, rater agreement
//! and a Bradley-Terry ranking of three or more arms.
//!
//! A judgment of arms A and B is scored for B: 1 when B wins, 0 when A wins, ½ for a
//! tie. Judgments of one item by one rater form a group; a group holding both
//! presentation orders is a swapped pair, and its mean cancels the position effect.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::agreement;
use crate::bootstrap::{self, MIN_UNITS};
use crate::checks::{Finding, Overrides};
use crate::interval::wilson_two_sided;
use crate::numeric::{Rng, Running, mean, sum};
use crate::records::{Preference, PreferenceOutcome};

/// One judgment of the pair (A, B), normalised to B's side.
#[derive(Debug, Clone, PartialEq)]
pub struct Judged<'a> {
    /// ID of the item the pair was judged on.
    pub item_id: &'a str,
    /// ID of the rater (human or model) who judged.
    pub rater_id: &'a str,
    /// Which re-grading of the same output pair this is; 0 when not recorded.
    pub grade_replicate: u32,
    /// Whether A was shown on the left.
    pub a_left: bool,
    /// 1 when B won, 0 when A won, ½ for a tie.
    pub score: f64,
}

impl Judged<'_> {
    /// Whether the left side won; `None` for a tie.
    fn left_won(&self) -> Option<bool> {
        (self.score != 0.5).then_some((self.score == 0.0) == self.a_left)
    }
}

/// The judgments comparing runs `a` and `b`, in either order, that have an outcome.
pub fn normalise<'a>(
    preferences: impl IntoIterator<Item = &'a Preference>,
    a: &str,
    b: &str,
) -> Vec<Judged<'a>> {
    preferences
        .into_iter()
        .filter_map(|p| {
            let a_left = match (p.left_run_id.as_str(), p.right_run_id.as_str()) {
                (l, r) if l == a && r == b => true,
                (l, r) if l == b && r == a => false,
                _ => return None,
            };
            let left_score = match p.outcome? {
                PreferenceOutcome::Left => 1.0,
                PreferenceOutcome::Right => 0.0,
                PreferenceOutcome::Tie => 0.5,
            };
            Some(Judged {
                item_id: &p.item_id,
                rater_id: &p.rater_id,
                grade_replicate: p.grade_replicate.unwrap_or(0),
                a_left,
                score: if a_left { 1.0 - left_score } else { left_score },
            })
        })
        .collect()
}

type Groups<'j, 'a> = BTreeMap<(&'a str, &'a str), Vec<&'j Judged<'a>>>;

fn groups<'j, 'a>(judged: &'j [Judged<'a>]) -> Groups<'j, 'a> {
    let mut g: Groups<'j, 'a> = BTreeMap::new();
    for j in judged {
        g.entry((j.item_id, j.rater_id)).or_default().push(j);
    }
    for v in g.values_mut() {
        v.sort_by_key(|j| (j.grade_replicate, !j.a_left));
    }
    g
}

fn swapped(group: &[&Judged<'_>]) -> bool {
    group.iter().any(|j| j.a_left) && group.iter().any(|j| !j.a_left)
}

/// B's win rate over A.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WinRate {
    /// Mean score of B against A, from 0 to 1; ½ means no preference.
    pub value: f64,
    /// Percentile bootstrap interval over items; `None` with too few items.
    pub ci: Option<(f64, f64)>,
    /// Method of the interval: `bootstrap`, or `too_few_units` without one.
    pub interval: &'static str,
    /// Number of items judged.
    pub items: usize,
    /// Number of judgments.
    pub judgments: usize,
    /// Number of (item, rater) groups judged in both presentation orders.
    pub swapped_pairs: usize,
}

/// The win rate: each (item, rater) group's mean score, averaged over the raters of
/// an item and then over items; the interval resamples items.
pub fn win_rate(judged: &[Judged<'_>], level: f64, b: u32, seed: u64) -> Option<WinRate> {
    let groups = groups(judged);
    let mut by_item: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for ((item, _), g) in &groups {
        by_item
            .entry(item)
            .or_default()
            .extend(mean(g.iter().map(|j| j.score)));
    }
    let items: Vec<f64> = by_item
        .values()
        .filter_map(|v| mean(v.iter().copied()))
        .collect();
    let value = mean(items.iter().copied())?;
    let (ci, interval) = if items.len() < MIN_UNITS {
        (None, "too_few_units")
    } else {
        let mut rng = Rng::new(seed);
        let (reps, _) = bootstrap::replicates(items.len(), None, b, &mut rng, |drawn| {
            mean(drawn.iter().map(|&i| items[i]))
        });
        (bootstrap::percentile_interval(reps, level), "bootstrap")
    };
    Some(WinRate {
        value,
        ci,
        interval,
        items: items.len(),
        judgments: judged.len(),
        swapped_pairs: groups.values().filter(|g| swapped(g)).count(),
    })
}

/// Position effects, from the groups that hold both presentation orders.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PositionBias {
    /// Number of (item, rater) groups judged in both presentation orders.
    pub swapped_pairs: usize,
    /// Share of swapped pairs whose first judgment in each order agree.
    pub order_consistency: Option<f64>,
    /// Share of the decided (not tied) judgments in swapped pairs won by the left side.
    pub left_win_rate: Option<f64>,
    /// Interval of `left_win_rate`.
    pub ci: Option<(f64, f64)>,
    /// Number of decided (not tied) judgments in swapped pairs.
    pub decided: usize,
}

/// The position effects among the judgments, with an interval at `level`.
pub fn position_bias(judged: &[Judged<'_>], level: f64) -> PositionBias {
    let groups = groups(judged);
    let pairs: Vec<&Vec<&Judged<'_>>> = groups.values().filter(|g| swapped(g)).collect();
    let consistent = pairs
        .iter()
        .filter(|g| {
            let first = |a_left: bool| g.iter().find(|j| j.a_left == a_left).map(|j| j.score);
            first(true) == first(false)
        })
        .count();
    let decided: Vec<bool> = pairs
        .iter()
        .flat_map(|g| g.iter().filter_map(|j| j.left_won()))
        .collect();
    let left = decided.iter().filter(|w| **w).count() as f64;
    let n = decided.len() as f64;
    PositionBias {
        swapped_pairs: pairs.len(),
        order_consistency: (!pairs.is_empty()).then(|| consistent as f64 / pairs.len() as f64),
        left_win_rate: (n > 0.0).then(|| left / n),
        ci: (n > 0.0).then(|| wilson_two_sided(left, n, level)),
        decided: decided.len(),
    }
}

/// `no_swapped_pairs` without a swapped pair, `position_bias` when the left side's
/// win-rate interval excludes ½.
pub fn checks(bias: &PositionBias, overrides: &Overrides) -> Vec<Finding> {
    let mut out = Vec::new();
    if bias.swapped_pairs == 0 {
        out.push(Finding::new(
            "no_swapped_pairs",
            "no item has judgments in both presentation orders, so position bias cannot be measured",
            overrides,
        ));
    }
    if let (Some(rate), Some((lo, hi))) = (bias.left_win_rate, bias.ci)
        && (hi < 0.5 || lo > 0.5)
    {
        out.push(Finding::new(
            "position_bias",
            format!(
                "the left side wins {rate:.2} of decided swapped judgments (interval {lo:.2} to {hi:.2})"
            ),
            overrides,
        ));
    }
    out
}

/// Agreement of two raters on the items both judged, on the three values A, B, tie.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RaterPair {
    /// The two raters, in sorted order.
    pub raters: (String, String),
    /// Number of items both judged.
    pub items: usize,
    /// Cohen's κ over the values A, B and tie.
    pub cohen_kappa: Option<f64>,
    /// Gwet's AC1 over the values A, B and tie.
    pub gwet_ac1: Option<f64>,
}

/// Each rater's normalised verdict per item (the side of the mean score of the group),
/// then κ and AC1 for every pair of raters with a shared item.
pub fn rater_agreement(judged: &[Judged<'_>]) -> Vec<RaterPair> {
    let mut verdicts: BTreeMap<&str, BTreeMap<&str, f64>> = BTreeMap::new();
    for ((item, rater), g) in groups(judged) {
        if let Some(m) = mean(g.iter().map(|j| j.score)) {
            let v = match m.partial_cmp(&0.5) {
                Some(std::cmp::Ordering::Greater) => 1.0,
                Some(std::cmp::Ordering::Less) => 0.0,
                _ => 0.5,
            };
            verdicts.entry(rater).or_default().insert(item, v);
        }
    }
    let raters: Vec<&str> = verdicts.keys().copied().collect();
    let mut out = Vec::new();
    for (i, a) in raters.iter().enumerate() {
        for b in &raters[i + 1..] {
            let pairs: Vec<(f64, f64)> = verdicts[a]
                .iter()
                .filter_map(|(item, va)| verdicts[b].get(item).map(|vb| (*va, *vb)))
                .collect();
            if pairs.is_empty() {
                continue;
            }
            // A fixed 3×3 table (A, tie, B) keeps κ defined when a rater uses one value.
            let mut table = vec![vec![0.0; 3]; 3];
            for (va, vb) in &pairs {
                table[(va * 2.0) as usize][(vb * 2.0) as usize] += 1.0;
            }
            out.push(RaterPair {
                raters: ((*a).to_owned(), (*b).to_owned()),
                items: pairs.len(),
                cohen_kappa: agreement::cohen_kappa(&table),
                gwet_ac1: agreement::gwet_ac1(&table),
            });
        }
    }
    out
}

/// One comparison of arms `i` and `j`, scored for `j` (1 win, 0 loss, ½ tie).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comparison {
    /// Index of the first arm.
    pub i: usize,
    /// Index of the second arm, for which the comparison is scored.
    pub j: usize,
    /// Score for arm `j`: 1 win, 0 loss, ½ tie.
    pub score: f64,
}

/// Why arms cannot be ranked.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RankingError {
    /// Fewer than two arms were compared.
    #[error("a ranking needs preferences between at least two arms")]
    TooFewArms,
    /// The arms split into groups that were never compared with each other.
    #[error("the comparison graph of the arms is not connected")]
    Disconnected,
    /// An arm wins or loses every comparison, so its strength has no finite estimate.
    #[error("the strengths have no finite estimate (an arm wins or loses every comparison)")]
    NotEstimable,
}

/// A Bradley-Terry fit: log-strengths centred to mean 0 and, when there are ties, the
/// Davidson tie parameter ν.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fit {
    /// Log-strength of each arm, centred to mean 0.
    pub log_strength: Vec<f64>,
    /// Davidson tie parameter ν; present when ties occur.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tie: Option<f64>,
}

fn connected(arms: usize, comparisons: &[Comparison]) -> bool {
    let mut parent: Vec<usize> = (0..arms).collect();
    fn root(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for c in comparisons {
        let (a, b) = (root(&mut parent, c.i), root(&mut parent, c.j));
        parent[a] = b;
    }
    let r = root(&mut parent, 0);
    (0..arms).all(|a| root(&mut parent, a) == r)
}

/// Solve A·x = b by Gaussian elimination with partial pivoting.
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|&x, &y| a[x][c].abs().total_cmp(&a[y][c].abs()))?;
        if a[p][c].abs() < 1e-300 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        let pivot = a[c].clone();
        for r in c + 1..n {
            let f = a[r][c] / pivot[c];
            for (x, y) in a[r][c..].iter_mut().zip(&pivot[c..]) {
                *x -= f * y;
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| a[r][k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    Some(x)
}

const MAX_ITERATIONS: usize = 200;
/// Log-strengths beyond this mean the maximum lies at infinity.
const MAX_LOG_STRENGTH: f64 = 30.0;

/// The maximum-likelihood Bradley-Terry fit, with Davidson's tie parameter when any
/// comparison is a tie. Each comparison is a softmax over the logits θ_i, θ_j and
/// log ν + (θ_i + θ_j)/2; Newton's method on (θ_1 … θ_{m−1}, log ν) with θ_0 = 0.
pub fn bradley_terry(arms: usize, comparisons: &[Comparison]) -> Result<Fit, RankingError> {
    if arms < 2 {
        return Err(RankingError::TooFewArms);
    }
    if !connected(arms, comparisons) {
        return Err(RankingError::Disconnected);
    }
    let ties = comparisons.iter().any(|c| c.score == 0.5);
    let dim = arms - 1 + usize::from(ties);
    // The likelihood depends only on how often each ordered pair had each outcome, so
    // every pass runs over those counts rather than over the comparisons.
    let mut counts: BTreeMap<(usize, usize, u64), (Comparison, f64)> = BTreeMap::new();
    for c in comparisons {
        counts
            .entry((c.i, c.j, c.score.to_bits()))
            .or_insert((*c, 0.0))
            .1 += 1.0;
    }
    let groups: Vec<(Comparison, f64)> = counts.into_values().collect();
    // A comparison touches at most three parameters: θ_i, θ_j (θ_0 is fixed at 0) and
    // log ν. `slots` lists them; each logit is a coefficient per slot.
    let local = |c: &Comparison| -> ([Option<usize>; 3], [[f64; 3]; 3], usize) {
        let slots = [
            c.i.checked_sub(1),
            c.j.checked_sub(1),
            ties.then_some(dim - 1),
        ];
        let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.5, 1.0]];
        for (s, slot) in slots.iter().enumerate() {
            if slot.is_none() {
                for row in &mut v {
                    row[s] = 0.0;
                }
            }
        }
        let observed = if c.score == 0.0 {
            0
        } else if c.score == 1.0 {
            1
        } else {
            2
        };
        (slots, v, observed)
    };
    // Logits and softmax probabilities of one comparison at x.
    let softmax =
        |slots: &[Option<usize>; 3], v: &[[f64; 3]; 3], x: &[f64]| -> ([f64; 3], [f64; 3], usize) {
            let n = if ties { 3 } else { 2 };
            let mut s = [f64::NEG_INFINITY; 3];
            for (k, sk) in s.iter_mut().enumerate().take(n) {
                *sk = (0..3)
                    .filter_map(|d| slots[d].map(|g| v[k][d] * x[g]))
                    .sum();
            }
            let max = s[..n].iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let total: f64 = s[..n].iter().map(|v| (v - max).exp()).sum();
            let mut p = [0.0; 3];
            for k in 0..n {
                p[k] = (s[k] - max).exp() / total;
            }
            let lse = max + total.ln();
            for sk in &mut s[..n] {
                *sk -= lse;
            }
            (s, p, n)
        };
    let loglik = |x: &[f64]| -> f64 {
        sum(groups.iter().map(|(c, w)| {
            let (slots, v, o) = local(c);
            w * softmax(&slots, &v, x).0[o]
        }))
    };
    let mut x = vec![0.0; dim];
    let mut converged = false;
    for _ in 0..MAX_ITERATIONS {
        let mut g = vec![Running::default(); dim];
        let mut h = vec![vec![Running::default(); dim]; dim];
        for (c, w) in &groups {
            let (slots, v, observed) = local(c);
            let (_, p, n) = softmax(&slots, &v, &x);
            let mut mu = [0.0; 3];
            for (d, m) in mu.iter_mut().enumerate() {
                *m = (0..n).map(|k| p[k] * v[k][d]).sum();
            }
            for d in 0..3 {
                let Some(gd) = slots[d] else { continue };
                g[gd].add(w * (v[observed][d] - mu[d]));
                for e in 0..3 {
                    let Some(ge) = slots[e] else { continue };
                    let second: f64 = (0..n).map(|k| p[k] * v[k][d] * v[k][e]).sum();
                    h[gd][ge].add(w * (second - mu[d] * mu[e]));
                }
            }
        }
        let g: Vec<f64> = g.iter().map(Running::total).collect();
        let h = h
            .iter()
            .map(|r| r.iter().map(Running::total).collect())
            .collect();
        let step = solve(h, g).ok_or(RankingError::NotEstimable)?;
        let before = loglik(&x);
        let mut t = 1.0;
        let next = loop {
            let cand: Vec<f64> = x.iter().zip(&step).map(|(a, s)| a + t * s).collect();
            if loglik(&cand) >= before - 1e-12 || t < 1e-8 {
                break cand;
            }
            t /= 2.0;
        };
        let change = next
            .iter()
            .zip(&x)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        x = next;
        if x.iter().any(|v| v.abs() > MAX_LOG_STRENGTH) {
            return Err(RankingError::NotEstimable);
        }
        if change < 1e-13 {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err(RankingError::NotEstimable);
    }
    let mut log_strength: Vec<f64> = std::iter::once(0.0)
        .chain(x[..arms - 1].iter().copied())
        .collect();
    let centre = sum(log_strength.iter().copied()) / arms as f64;
    for v in &mut log_strength {
        *v -= centre;
    }
    Ok(Fit {
        log_strength,
        tie: ties.then(|| x[dim - 1].exp()),
    })
}

/// An arm's place in the ranking.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ranked {
    /// Name of the arm.
    pub arm: String,
    /// Position in the ranking, 1 for the strongest.
    pub rank: usize,
    /// Bradley-Terry log-strength, centred to mean 0.
    pub log_strength: f64,
    /// Percentile bootstrap interval of the log-strength; `None` without enough valid
    /// resamples.
    pub ci: Option<(f64, f64)>,
}

/// A Bradley-Terry ranking of arms with bootstrap intervals.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ranking {
    /// The arms, strongest first.
    pub arms: Vec<Ranked>,
    /// Davidson tie parameter ν; present when ties occur.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tie: Option<f64>,
    /// Method of the intervals: `bootstrap`, or `too_few_units` without one.
    pub interval: &'static str,
    /// Resamples whose refit failed (an arm dropped out or won everything); the
    /// intervals come from the others.
    pub failed_resamples: u32,
}

/// The ranking of arms from preference records, strongest first, with percentile
/// bootstrap intervals of the log-strengths over items (refitting each resample).
pub fn ranking<'a>(
    preferences: impl IntoIterator<Item = &'a Preference>,
    level: f64,
    b: u32,
    seed: u64,
) -> Result<Ranking, RankingError> {
    let mut by_item: BTreeMap<&str, Vec<(&str, &str, f64)>> = BTreeMap::new();
    let mut names = BTreeSet::new();
    for p in preferences {
        let Some(o) = p.outcome else { continue };
        let score = match o {
            PreferenceOutcome::Left => 0.0,
            PreferenceOutcome::Right => 1.0,
            PreferenceOutcome::Tie => 0.5,
        };
        names.insert(p.left_run_id.as_str());
        names.insert(p.right_run_id.as_str());
        by_item
            .entry(&p.item_id)
            .or_default()
            .push((&p.left_run_id, &p.right_run_id, score));
    }
    let names: Vec<&str> = names.into_iter().collect();
    let index = |n: &str| names.binary_search(&n).unwrap_or(0);
    let items: Vec<Vec<Comparison>> = by_item
        .values()
        .map(|cs| {
            cs.iter()
                .map(|(l, r, s)| Comparison {
                    i: index(l),
                    j: index(r),
                    score: *s,
                })
                .collect()
        })
        .collect();
    let all: Vec<Comparison> = items.iter().flatten().copied().collect();
    let fit = bradley_terry(names.len(), &all)?;
    let mut failed_resamples = 0;
    let (cis, interval) = if items.len() < MIN_UNITS {
        (vec![None; names.len()], "too_few_units")
    } else {
        let mut rng = Rng::new(seed);
        let mut draws: Vec<Vec<f64>> = vec![Vec::new(); names.len()];
        for _ in 0..b {
            let drawn: Vec<usize> = (0..items.len())
                .map(|_| rng.below(items.len() as u64) as usize)
                .collect();
            let cs: Vec<Comparison> = drawn
                .iter()
                .flat_map(|&i| items[i].iter().copied())
                .collect();
            match bradley_terry(names.len(), &cs) {
                Ok(f) => {
                    for (d, v) in draws.iter_mut().zip(f.log_strength) {
                        d.push(v);
                    }
                }
                Err(_) => failed_resamples += 1,
            }
        }
        (
            draws
                .into_iter()
                .map(|d| bootstrap::percentile_interval(d, level))
                .collect(),
            "bootstrap",
        )
    };
    let mut arms: Vec<Ranked> = names
        .iter()
        .zip(&fit.log_strength)
        .zip(cis)
        .map(|((n, s), ci)| Ranked {
            arm: (*n).to_owned(),
            rank: 0,
            log_strength: *s,
            ci,
        })
        .collect();
    arms.sort_by(|a, b| b.log_strength.total_cmp(&a.log_strength));
    for (k, a) in arms.iter_mut().enumerate() {
        a.rank = k + 1;
    }
    Ok(Ranking {
        arms,
        tie: fit.tie,
        interval,
        failed_resamples,
    })
}
