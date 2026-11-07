# Metric definitions

Version `metrics/1` (`SPEC_VERSION` in `crates/evalsig-core/src/lib.rs`; results carry it
as `provenance.spec_version`). This page defines every number evalsig outputs: the
formulas, the interval method behind each one, and what each command computes today.
The dashboard's "how to read" text comes from it. Commands and options are listed in the
generated CLI reference, `docs/cli.md`; terms follow `docs/glossary.md`.

Reference values for most formulas are in `tests/fixtures/reference-values.json`; the
Rust implementation matches them within an absolute error of 1e-9. Each section names its
case, or says "reference case: none".

The `latent` reference is left out of v1 (D-016, §9.2).

## 0. Conventions

- **Value semantics** (`run.value_semantics`). A judgment's value means one of two things,
  and the usable metrics differ (§0.2).
  - `decision`: the system's decision on an item (positive or negative, a level, a score).
    It is compared with reference labels to give accuracy, hit rate, false alarm rate,
    precision, ROC and AUC.
  - `score`: a grade of an output (correct or not, pass or fail, points). The grade is the
    result, so no reference label is needed. A binary score run reports `pass_rate`; a
    numeric one reports `mean_score` and, when a threshold t is set, `rate_at_least`.
- **The counting unit is the item.** Replicates are observations inside an item and never
  add to the count (§6). When items belong to a `cluster`, intervals resample clusters
  (§8).
- **Estimand.** The default is the performance of one production run, `single_shot` (§6.1).
- **Positive definition and decision rule are separate.** `positive` says which reference
  label values count as positive. `decision` says from which judgment value the system
  counts as positive (`score_at_least = 0.5`, `ordinal_at_least = 2`, `equals = true`). A
  judgment the rule cannot read (a value of another type) counts as a missing judgment with
  reason `invalid_output`.
- **Missing values** are handled per reason, and their counts are always reported (§0.1).
  Every report and comparison carries `n_missing`, `missing_by_reason` and
  `missing_policy`; each metric carries `n`, the denominator it used.
- **Weights.** Ratio metrics are weighted: an item contributes its `weight`, multiplied by
  1/p when it has a `label_inclusion_prob` p and a reference (§9.9). An item with a weight
  other than 1 makes the metric's interval a cluster bootstrap (§1.1). Kish's effective
  sample size is not used in any interval.
- **Level and direction.** The default is a two-sided 95% interval (`level = 0.95`). Each
  gate condition states `sided` (`one` or `two`) (§11.1).
- **Interval method.** §1.1 chooses it. No command reads the `interval` setting.
- **Quantiles.** Normal and Beta quantiles come from `statrs` in double precision (D-009).
- **Random numbers.** ChaCha20 seeded from a `u64` seed (D-009). Uniform integers use
  Lemire's unbiased method and uniform doubles take 53 bits, both written in evalsig. Every
  output that used randomness records its seed.
- **Rounding and determinism.** Numbers in an output are rounded to 10 decimal places.
  Sums use Neumaier compensation in a fixed iteration order, so results do not depend on
  threads or map order (D-009).

### 0.1 Missing values

Missing reasons split into the system's side and the measurement's side.

| Side | Reason codes | Default `missing_policy` |
|---|---|---|
| system | `error`, `timeout`, `refused`, `invalid_output`, `limit` | `worst`: the item counts as judged worst (§0.1.1) |
| measurement | `grader_failed`, `not_run`, `invalidated`, `excluded`, `other` | `drop`: removed from the denominator, counted in `n_missing` |

- **Why system failures are not dropped.** Refusals and timeouts cluster on hard items.
  Dropping them raises accuracy and lets a candidate that refuses hard items win a
  comparison.
- **Missing bounds.** When any system-side judgment is missing, the metric also reports
  `missing_bounds`: the value with system-side gaps filled worst and filled best (Manski
  bounds). Measurement-side gaps follow the policy in both fills. In a comparison the
  bounds pair a worst-filled candidate with a best-filled baseline and the reverse.
  `precision` has no worst fill and no bounds.
- **Configurable.** The `[missing]` setting picks `worst`, `best` or `drop` for each
  reason. The resolved table is output as `missing_policy`.
- **Differing missing rates.** When the 95% Newcombe interval of the difference in
  system-side missing rates between two arms excludes 0, `compare` warns
  `missing_rate_differs`.
- **The gate does not trust reason codes.** A candidate's records are the candidate's own
  claim, so on the holdout every missing judgment is `worst` whatever its reason, and the
  `missing_rate` condition counts every missing judgment (§11.2).

#### 0.1.1 The worst judgment per metric

| Metric | Worst fill |
|---|---|
| `accuracy` | the side opposite the reference majority: judged negative when the reference share is at least ½, otherwise judged positive |
| `hit_rate` (H) | judged negative |
| `false_alarm_rate` (F) | judged positive |
| `precision` | judged negative; no bounds |
| `pass_rate`, `rate_at_least` | fail; below t |
| `mean_score` | the lowest score seen in the run |
| AUC | not filled: an item with no judgment value is left out |
| win rate | not filled: a preference with no outcome is left out |

With a soft reference the item keeps its weight s (or 1 − s) in H and F; only the judgment
is filled.

### 0.2 Value semantics and usable metrics

| Metric | `decision` (needs reference labels) | `score` (no reference labels) |
|---|---|---|
| accuracy, H, F, precision, ROC, AUC, calibration, feature likelihood ratios, expected cost, transition table | yes | no |
| `pass_rate` | no | yes (binary scores) |
| `mean_score`, `rate_at_least` | no | yes |
| replicate disagreement, k-run rules | yes | yes |
| paired comparison, verdict | yes | yes |
| agreement and meta-evaluation | yes | no (import the grader's judgments as a `decision` run to evaluate the grader) |

- **Correct/incorrect generation tasks** (Inspect's C/I, promptfoo's pass) are `score`
  runs. The adapter does not invent reference labels, turns the value into a pass or fail
  and requires `run.grader`.
- **A `score` result depends on its grader.** Rewriting the grader changes the result. The
  grader version check (§11.2) stops mix-ups; it cannot stop a grader being altered.
- Reference case: `score`.

## 1. Intervals for rates

A rate is `k/n`; `k` and `n` may be fractional (weights, soft references, replicate
shares).

| Method | Formula |
|---|---|
| Wilson (two-sided, display) | centre (p + z²/2n) / (1 + z²/n), half-width z·√(p(1−p)/n + z²/4n²) / (1 + z²/n), clipped to [0, 1] |
| Clopper-Pearson (exact) | upper = Beta⁻¹(1−α; k+1, n−k), and 1 when k = n; lower = Beta⁻¹(α; k, n−k+1), and 0 when k = 0 |
| Jeffreys | Beta⁻¹(α/2; k+½, n−k+½) and Beta⁻¹(1−α/2; k+½, n−k+½) |
| Newcombe hybrid score | for p_a − p_b of two independent rates, from the two Wilson intervals (§7.3) |

Every interval function returns the vacuous [0, 1] when n = 0.

- **What reports show.** A displayed rate (accuracy, H, F, precision, `pass_rate`,
  `rate_at_least`) gets a Wilson interval when every unit is one item with weight 1, one
  replicate and an integral contribution, and a cluster bootstrap (§8) otherwise.
- **What pass/fail decisions use.** In `gate` and `check` the same rates take the exact
  Clopper-Pearson interval on the effective number of items: n_eff = (Σ d_c)² / Σ d_c²,
  with d_c the summed denominators of cluster c (an item without a cluster is its own).
  n_eff is the item count for independent items of weight 1, the cluster count for equal
  clusters, and smaller for unequal weights, clusters or soft references; the event count
  is p̂ · n_eff. The `missing_rate` condition and the replicate-disagreement upper bound
  (§6.2) use the exact method too. The library also provides Jeffreys and a method
  chooser, which no command calls.
- **Why exact.** With no events the Wilson one-sided upper bound is smaller than the
  exact one, 1.645²/(n + 1.645²) against 1 − α^(1/n) at α = 0.05, and a percentile
  bootstrap returns 0, which would pass a rate condition on no evidence. At n = 300 and a
  true rate of 0.01 (0.005), the bootstrap's one-sided 99% upper bound falls below the
  true rate in 5.0% (22.5%) of samples, each one with no event observed, and
  Clopper-Pearson's never does (`scripts/sim_rare_rate_bounds.py`). Items needed to
  claim a rate below u with no events: `docs/sample-size.md`, table 1.
- Reference case: `proportion_interval`.

### 1.1 Choosing the interval method (`interval = "auto"`)

| Metric | Replicates, clusters or weights | Method |
|---|---|---|
| one rate | none | Wilson (display), Clopper-Pearson (`gate`, `check`) |
| one rate | present | cluster bootstrap (display), Clopper-Pearson on n_eff (`gate`, `check`) |
| paired difference of rates (accuracy, H, F, `pass_rate`, `rate_at_least`) | none, and every arm's items independent | Tango score interval (§7) |
| paired difference of rates | present | cluster bootstrap (§8) |
| any other paired difference (`precision`, `mean_score`), win rate, a difference of unpaired rates with clusters | n/a | bootstrap |
| unpaired difference of rates | none | Newcombe (§7.3) |
| unpaired difference of mean scores | unit-weight single items | Welch's t (§7.3) |
| AUC | binary reference, one score per item | DeLong; Obuchowski's variance when items share clusters (§3.2) |
| AUC | soft or weighted reference, or an AUC of 0 or 1 | cluster bootstrap |

- A bootstrap interval needs at least 5 resampling units (clusters, or items without
  clusters). With fewer, no interval is given and the method is `too_few_units`.
- A gate draft that names an interval method other than `auto` is refused at drafting.

## 2. Signal detection metrics of binary decisions

The counts are weighted counts of reference class by decision: hits, misses, false alarms
and correct rejections. A soft reference splits an item into s positive and 1 − s
negative; replicates enter as the share of positive judgments (§6.1).

- **Raw rates.** H = hits / positives, F = false alarms / negatives, accuracy = (hits +
  correct rejections) / total. A rate with an empty denominator is undefined.
- **Precision.** hits / (hits + false alarms), in the test set. With a production
  prevalence π given, `precision_at_prevalence` = πH / (πH + (1−π)F).
- **Corrected rates.** H′ = (hits + ½) / (positives + 1) and F′ = (false alarms + ½) /
  (negatives + 1), the log-linear correction on every cell (Hautus 1995).
- **Sensitivity index and criterion.** d′ = z(H′) − z(F′) and c = −(z(H′) + z(F′)) / 2,
  with z the normal quantile function.
- **The assumption.** One ROC point cannot check the model, so d′ and c assume
  equal-variance normal distributions. The library names it `equal_variance_normal`. A
  check needs ordinal or score output (§3).
- **Corrected quantities depend on counts.** Multiplying every count by k leaves the raw
  rates unchanged and moves d′ and c slightly. Flipping which class is positive flips the
  sign of d′.
- **Where they appear.** `report` and `compare` output accuracy, H, F, precision and
  `precision_at_prevalence`. `report`'s detail tier adds `sdt` with `dprime`, `criterion`
  and `assumption`, from the same weighted counts as H and F; they have no interval.
- Reference case: `sdt_binary`.

## 3. Ordinal and score output

### 3.1 Empirical ROC and AUC

Applies to a `decision` run whose output is ordinal or score and that has a reference.
Levels count up from the weakest, and a larger value is more positive.

- **ROC.** One threshold at each distinct score, from the strongest. The point for a
  threshold is (false alarm rate, hit rate) among the items at or above it, with (0, 0) and
  (1, 1) at the ends.
- **AUC.** The Mann-Whitney statistic Σ w_i w_j ψ(i, j) / Σ w_i w_j, with ψ = 1 when the
  positive scores higher, ½ on a tie and 0 below. It equals the area under the
  empirical ROC. An item with replicates contributes the mean of its replicate scores.
- **Soft or latent reference.** With the reference share s_i of item i, AUC_soft =
  Σ_{i,j} s_i (1 − s_j) ψ(i, j) / Σ_{i,j} s_i (1 − s_j), where the pair i = j is included
  with ψ = ½. It is the probability that a random rater's positive item outranks a random
  rater's negative one.
- A binary decision run has no ROC.
- **Few levels.** The empirical AUC makes no assumption, but with few levels it is smaller
  than the AUC of an underlying smooth curve. The output carries the counts per level.
- Reference case: `ordinal_roc`.

### 3.2 AUC intervals (DeLong)

- **Structural components.** For positive item i and negative item j, ψ(i, j) as above;
  V10_i = (1/n_neg) Σ_j ψ(i, j) and V01_j = (1/n_pos) Σ_i ψ(i, j).
- **Variance.** Var(AUC) = S10/n_pos + S01/n_neg, with S10 and S01 the sample variances of
  V10 and V01. It needs at least two positives and two negatives.
- **Interval.** Normal on the logit scale, mapped back: logit(AUC) ± z·√Var / (AUC(1 − AUC)).
  At an AUC of 0 or 1 the variance degenerates and a cluster bootstrap is used.
- **Clusters.** Components are summed per cluster and the variance combines the between
  cluster terms S10, S01 and S11 (Obuchowski 1997); the output names the method
  `obuchowski`.
- **Soft or weighted reference.** No DeLong interval; a cluster bootstrap of AUC_soft.
- **Paired AUC comparison.** `evalsig_core::roc::delong_paired` computes Var(ΔAUC) from the
  covariance of the components (DeLong, DeLong and Clarke-Pearson 1988). No command reports
  ΔAUC.
- Reference cases: `delong`, `auc_clustered`.

### 3.3 z-ROC

For an ordinal run with a 0/1 reference, the report carries the points (z(F′), z(H′)) at
each threshold between levels, with the correction of §2, and the dashboard draws them.
The report states no slope. d_a and an unequal-variance fit are not computed: the few
points are not independent, so a least-squares fit would be biased.

### 3.4 Calibration

For a score run whose scores all lie in [0, 1]:

- **Brier score.** The mean of (p_i − y_i)², with y_i the reference share.
- **Reliability bins.** Ten equal-width bins, [b/10, (b+1)/10) with the last including 1.
  Each non-empty bin reports its count, mean score and positive share.
- Reference case: `calibration`.

## 4. Cost and decision threshold

`evalsig threshold` computes these from a `[cost]` setting: a matrix with one row for each
reference class (negative, positive) and one column for each judgment level, the
production prevalence π, and optionally a range of the cost ratio.

- **Expected cost.** EC = π Σ_j P(j | positive) C[positive][j] + (1 − π) Σ_j P(j | negative)
  C[negative][j]. π is the production prevalence, not the test set's share. A soft
  reference counts an item as s positive and 1 − s negative.
- **Optimal decision, binary.** Call an item positive when its posterior probability
  exceeds p* = C_FP / (C_FP + C_FN); in likelihood-ratio terms when LR exceeds LR* =
  (C_FP/C_FN)(1 − π)/π.
- **Optimal threshold, levels.** For each threshold t = 1 … k−1, calling levels at or above
  t positive has expected cost π((1−H)C_FN) + (1−π)(F·C_FP), with C_FP = 1 and C_FN = r.
  The command reports every threshold's cost and picks the cheapest, the lower one on a
  tie. r is `--ratio` or the matrix's corners, C[positive][lowest] / C[negative][highest].
- **Cost-ratio sensitivity.** With a range [r_lo, r_hi], the command reports whether the
  optimal threshold differs at the two ends. Given a second run, it also reports whether
  the difference in expected cost between the two runs changes sign across the range.
- **Not in `compare`.** Expected cost and its difference are not among the metrics of
  `compare`.
- Reference cases: `cost`, `misc`.

## 5. Feature likelihood ratios

For a feature that fired in a of n_pos positives and b of n_neg negatives. Counts are
weighted: replicate shares, and s_i and 1 − s_i under a soft reference. An item that does
not report a feature counts as not firing it. Only items with a reference count.

- **Point.** LR = (a/n_pos) / (b/n_neg). When a or b is 0, ½ is added to all four cells
  (Haldane-Anscombe), so the denominators become n_pos + 1 and n_neg + 1; the output marks
  the ratio `corrected`.
- **Interval.** Katz log method: SE = √(1/a − 1/n_pos + 1/b − 1/n_neg), interval
  LR·exp(±z·SE), on the corrected cells when a correction was made.
- **Class.** Decided on the multiplicity-adjusted interval below: `insufficient` (fired
  fewer than 10 times in all), then `uninformative` (the interval contains 1), `strong`
  (the lower end is at least 3), otherwise `informative`.
- **Posterior.** With a prevalence π: p = LR·o / (1 + LR·o), o = π/(1−π).
- **Many features at once.** The class uses an interval at the level 1 − max(R, 1)·α/m,
  with m the number of features, R the number of Benjamini-Hochberg rejections of the
  p-values of log LR = 0, and α = 1 − level (a false coverage rate adjustment, Benjamini
  and Yekutieli 2005). The unadjusted interval is output beside it.
- Reference cases: `feature_likelihood_ratio`, `feature_likelihood_ratio_bh`.

## 6. Replicates and reproducibility

Replicates are of two kinds: generation replicates (`gen_replicate`, a re-run of the system
on the same item) and grading replicates (`grade_replicate`, a re-grading of the same
output). Item i has K_i generation replicates; grading replicates of one output are
averaged first.

### 6.1 The estimand

For item i with c_i of K_i replicates positive (or correct):

| `estimand` | Meaning | Item value |
|---|---|---|
| `single_shot` (default) | one production run | c_i / K_i |
| `majority@k` | k runs combined by majority | Σ_{j > k/2} C(c_i, j) C(K_i − c_i, k − j) / C(K_i, k); a tie at even k counts ½ |
| `any@k` | positive if any of k runs is (pass@k for generation tasks) | 1 − C(K_i − c_i, k) / C(K_i, k) (Chen et al. 2021) |
| `all@k` | positive only if all k runs are (pass^k) | C(c_i, k) / C(K_i, k) |

- **Conditions.** A rule at k estimates only items with K_i ≥ k. Items with fewer
  replicates are left out and counted in `n_insufficient_replicates`. Replicates are
  assumed exchangeable. A `@k` estimand needs a binary decision rule (or a threshold).
- **Grading replicates under `@k`.** The grading replicates of one output are averaged
  and the output counts as positive when the average is at least ½.
- **Score output.** An item's value is its mean over replicates. `mean_score` is the mean
  of item values and `rate_at_least` is the mean over items of the share of an item's
  replicates at or above t.
- **Missing replicates.** Filled or dropped by §0.1 before the item value is formed.
- Reference case: `estimand_at_k`.

#### 6.1.1 Performance and cost as k varies (k-curve of `report`)

For k = 1 … min K_i, the report gives the metrics under `majority@k`, `any@k` and `all@k`
and, when judgments carry `cost`, the cost per item: k times the mean cost of one
judgment, for each cost key. It supports choosing how many generations to run in
production and how to combine them.

### 6.2 Reproducibility

- **Pairwise disagreement (default).** For each item, the share of its C(K_i, 2) replicate
  pairs whose judgments differ; the mean over items. It compares across different K.
  Items with fewer than two replicates are left out.
- **Items with any disagreement.** The share of items having at least one differing pair.
  It grows with K.
- **Grader reproducibility.** The same measures over the grading replicates of one output,
  reported separately from generation reproducibility.
- **Upper bound.** The report gives the exact one-sided upper bound (§1) of the pairwise
  rate, with n_eff = min(number of clusters, number of items with two or more replicates).
- **Identical replicates.** If every generation replicate of an item has the same output
  (`output_ref`), the check `identical_replicates` warns, because a provider cache may
  stand behind the replicates. At the gate the same finding stops the submission and cannot
  be overridden (§11.2).
- **Selecting replicates cannot be detected.** If the candidate's side records only its
  best replicates, the records do not show it. Intervals are as honest as the records.

### 6.3 Run-to-run variation

The run-to-run standard deviation of a metric is taken over R ≥ 2 runs of the same system
version. It is a diagnostic and never enters an interval: redrawing runs would count again
the variance that resampling items already holds (§8). The verdict rests on the interval
alone. `compare` computes it for the primary metric over the runs of the baseline's and the
chosen candidate's system versions, and warns `run_variance_exceeds_delta` when the larger
of the two exceeds the point difference.

## 7. Paired comparison

Arm A is the baseline and arm B the candidate, compared on the items both have. Items in
one arm only are counted in `n_unpaired`. An item whose input hash (`input_ref`) differs
between the arms is excluded and counted in `n_input_changed`, unless the check is
overridden. Differences are B − A, oriented so that positive is better (the sign flips for
`false_alarm_rate`). Both arms must share value semantics, output type and decision rule.

The metrics compared are those both arms support: accuracy, H, F, precision, `pass_rate`,
`mean_score`, `rate_at_least`.

- **Interval of a paired difference of rates** (accuracy, H, F, `pass_rate`,
  `rate_at_least`). Without replicates, clusters, weights or fractional contributions,
  Tango's score interval (Tango 1998) from the discordant counts: b (A wrong, B right),
  c (A right, B wrong) and n paired items. Otherwise a paired cluster bootstrap that
  resamples the same clusters in both arms and recomputes the ratio of totals in each.
  Precision and mean score use the bootstrap. Reference case: `paired_difference`.
- **p-value.** For the Tango case, the exact two-sided McNemar test: 2·P(X ≤ min(b, c)),
  X ~ Binomial(b + c, ½), capped at 1. Otherwise the bootstrap p-value of §8. Reference
  case: `mcnemar`.
- **Transition table.** The weighted counts of items that became hits (reference positive,
  judged positive in B only), lost hits, became false alarms and stopped being false
  alarms. A soft reference contributes s and 1 − s, so the counts are real numbers.
- **Missing bounds.** §0.1.
- **Multiple candidates.** With several candidates against one baseline, the output gives
  Holm-adjusted p-values of the primary metric across candidates and names the candidate
  with the largest primary difference. The best of several is biased upwards (a winner's
  curse), so the output says `selected_after_comparison` and warns; confirm the best
  candidate on another split. Holm is not applied across metrics or strata.
- **Delta metrics not compared.** Δd′, ΔAUC and ΔEC are not among the compared metrics.

### 7.1 The verdict rule

For a difference Δ (positive is better), its interval [L, U] and the minimum effect m
(`min_effect`, default 0), the first matching row applies:

| Code | Condition |
|---|---|
| `negligible` | m > 0, −m < L and U < m (an equivalence test; holds whichever way Δ points) |
| `worsened` | U < 0 |
| `improved` | L > 0 and Δ ≥ m |
| `negligible` | L > 0 but Δ < m (a real difference, practically small) |
| `indistinguishable` | none of the above |
| `insufficient_data` | no interval could be formed |

- m is set per metric in `[compare] min_effect`. With m = 0 nothing is `negligible` by the
  first row.

### 7.2 Reusing validation data

In an agent's improvement loop the same validation data is compared many times. Two
situations get different treatment.

- **Trying one candidate after another.**
  - Each `compare` on a validation split counts toward `validation_uses`, which the output
    shows. The count is of ledger entries for the same split hash (a `dev` split is not
    counted). Past `[compare] validation_warn_after` (default 20) the warning
    `validation_uses` says it is time to rebuild the split.
  - No correction is applied. Reused validation data makes the tests dependent, the
    assumptions of online false-discovery-rate control (LORD++ and similar) do not hold,
    and the level would fall below 1% from the first use so that `improved` would almost
    never appear: with LORD++'s default γ and q = 0.1 the level is 0.0027 at the first
    test and 0.0002 at the tenth, and at 0.0005 the smallest difference detectable with
    80% power on 300 items, 10% of them discordant, is 0.076 (`scripts/sim_lord.py`). Validation data is a tool for searching; error control belongs to the
    sealed holdout and its gate (§11).
  - The count and warning help a cooperative agent; they are not a guard, since the ledger
    lives on the agent's side.
- **Adding items and looking along the way** (sequential mode of `compare`). Paired items
  are added in an order shuffled by the seed (`order_seed`), and the output gives a
  confidence sequence for the mean difference, valid however often it is inspected.
  - Method: the predictable plug-in empirical-Bernstein sequence for a bounded mean
    (Waudby-Smith and Ramdas 2024, Theorem 2). Differences in [−1, 1] map to [0, 1]
    through x = (d + 1)/2; at each item t, the bet is λ_t = min(√(2 log(2/α) /
    (σ̂²_{t−1} · t · log(1 + t))), ½), the interval is Σλ_i x_i / Σλ_i ± (log(2/α) +
    Σ v_i ψ(λ_i)) / Σλ_i with v_i = 4(x_i − μ̂_{i−1})² and ψ(λ) = (−log(1−λ) − λ)/4, and
    each interval is intersected with all earlier ones.
  - It applies to a metric only when every item counts with denominator 1 and a value in
    [0, 1], so the metric is a plain mean: accuracy, H, F, `pass_rate`, `rate_at_least` and
    mean scores within [0, 1]. A soft or weighted reference has no sequence, and neither
    does precision. The verdict (§7.1) reads the sequence.
- Reference case: `confidence_sequence`.

### 7.3 Unpaired comparison and trend

- **Unpaired comparison** (`compare` in unpaired mode). For two sets of items that need not
  overlap (for example production logs of different weeks). An item present in both arms
  counts in both. Rates of 0/1 items use Newcombe's hybrid score interval (§1); it gives
  no p-value. The mean score of unit-weight single items uses Welch's t interval and its
  p-value. Everything else uses a bootstrap in which each arm resamples its own clusters,
  with at least 5 clusters per arm. The verdict rule is §7.1. Reference case:
  `unpaired_difference`.
- **Trend** (`trend`). The metrics and intervals of each period of `occurred_at` (day,
  week or month). Each period is compared with the baseline period (the first unless
  named) by the unpaired comparison. A period is `drifted` when the interval of the
  difference excludes 0, `stable` when it contains 0, and `insufficient_data` when no
  interval could be formed. Items without `occurred_at` are left out and reported.

## 8. Bootstrap

- **Only the top level is resampled.** The unit is the cluster when items have clusters
  and the item otherwise. Replicates, raters and runs are not resampled: an item's value
  (the replicate mean, the rater share) is fixed as an observation. Redrawing a lower
  level counts again the variance that redrawing the upper level already holds, and
  widens the interval too much. On 300 items with 3 replicates each, item resampling
  gives a standard error within 1% of the true one and 95.3% coverage of a 95% interval;
  redrawing the replicates as well gives 1.20 times the true error and 97.5%
  (`scripts/sim_nested_bootstrap.py`).
- **No stratified resampling.** Draws are plain draws of units. The library can keep
  stratum sizes, but no command uses it.
- **Paired comparison.** Both arms use the same draw. In an unpaired comparison each arm
  draws its own.
- **Interval.** Percentile interval with B = 2000 resamples by default, quantiles by linear
  interpolation between order statistics (type 7). A resample whose statistic is undefined
  (an empty denominator) is dropped. With fewer than 5 units no interval is given
  (`too_few_units`).
- **Resamples for a gate.** A gate condition judged at a one-sided or two-sided level with
  tail α_adj needs B ≥ 50/α_adj (20 000 for α_adj = 0.0025). Approval raises the recorded
  B to that number when the configured B is lower. At B = 2000 the Monte Carlo spread of
  the 0.0025 quantile is 0.15 standard errors of the difference. Under the null, with
  300 items 3% of them discordant, the one-sided percentile lower bound at α = 0.0025 is
  above 0 in 0.35% of samples against 0.06% for the exact McNemar test, which is why
  paired rates take Tango and McNemar where they apply (§7)
  (`scripts/sim_percentile_tail.py`).
- **p-value.** The differences are centred on 0 by subtracting the observed difference;
  the p-value is (1 + number of resampled differences at least as far from 0 as the observed
  one) / (1 + number of valid resamples). It cannot reach 0, and its resolution is
  1/(B + 1).
- **Tests.** The same seed gives the same interval, and a pinned sum of resamples matches
  on every platform. Values are not compared with another implementation because random
  number generators differ.

## 9. Reference label quality and graders

Human reference labels are noisy measurements too: people disagree, and one person changes
over time. evalsig therefore measures the noise, states the estimand, and compares systems
and people by the same rule.

### 9.1 Agreement

| Measure | Use | Formula |
|---|---|---|
| Cohen's κ | two raters, nominal | (p_o − p_e)/(1 − p_e) |
| quadratic weighted κ | two raters, ordered integer levels | 1 − Σ w·p_o / Σ w·p_e with w_ab = (a − b)²/(Q − 1)², over every level from the lowest to the highest seen |
| Gwet's AC1 | agreement that stays high under skewed prevalence | (p_a − p_e)/(1 − p_e), p_e = (1/(Q − 1)) Σ_q π_q(1 − π_q), π_q the share of category q over both raters |
| positive and negative agreement | which judgment the raters split on | for the table [[a, b], [c, d]] with a both positive: 2a/(2a + b + c) and 2d/(2d + b + c) |
| Spearman's ρ | monotone agreement of scores | Pearson correlation of midranks |
| Krippendorff's α | several raters, missing values | 1 − (n − 1) Σ o_ck δ²_ck / Σ n_c n_k δ²_ck from the coincidence matrix; nominal, ordinal and interval δ; library function only |

- **Why not κ alone.** At low prevalence κ is low even when raw agreement is high (the κ
  paradox). Two-class values get κ, AC1 and both specific agreements together; ordered
  values get weighted κ and ρ.
- **Same-rule comparison.** `agree` reports, for each human, grader or run, the mean κ
  against each human rater other than itself (leave-one-out), and likewise for a system's
  decisions against each human's decisions. Comparing the system with a majority or soft
  reference while people are compared one to one makes the system look better at equal
  accuracy, so every rater is compared only with other raters. With three raters and a
  system each right 85% of the time, κ is 0.47 between two raters, 0.59 for the system
  against the majority and 0.46 for the system against each rater
  (`scripts/sim_agreement_rule.py`).
- **Ceiling.** With a soft reference, `agree` also gives mean(max(s_i, 1 − s_i)), the
  accuracy no system can exceed.
- **Intervals.** `agree` outputs point values; it computes no interval for agreement.
- Reference cases: `agreement`, `agreement_prevalence_robust`, `agreement_ordinal`.

### 9.2 Building the reference (`reference.mode`)

When an item has labels from several raters, the mode says what the reference is. The
output states the mode used. Human labels are used when an item has any; an item whose
labels all come from models has a model-derived reference, and the check
`model_derived_reference` flags model labels. A rater's vote is the latest revision of their first label replicate; adjudications are not
votes.

| Mode | Reference | When to use |
|---|---|---|
| `single` | the adjudication (`label.kind = "adjudication"`) if there is one; otherwise as `majority` | a person decides the truth |
| `majority` | the majority of the votes; a tie counts ½ | three or more raters, few splits |
| `soft` (default) | the share s_i of positive votes, used as a probability | the split itself carries meaning |
| `latent` | not available | see below |

- **Reading `soft`.** Every metric is the value against one rater drawn at random.

| Metric | Definition under `soft` |
|---|---|
| H, F | H = Σ w s_i y_i / Σ w s_i and F = Σ w (1 − s_i) y_i / Σ w (1 − s_i), y_i the system's value (§6.1) |
| accuracy | Σ w [y_i s_i + (1 − y_i)(1 − s_i)] / Σ w |
| precision | Σ w s_i y_i / Σ w y_i |
| AUC | AUC_soft (§3.1), bootstrap interval |
| d′, c | from H and F as in §2 |
| expected cost, likelihood ratios | items weighted s_i and 1 − s_i (§4, §5) |
| McNemar, transition table | bootstrap p-value; real-number counts (§7) |
| `ceiling` | mean(max(s_i, 1 − s_i)) |
| missing values | §0.1.1 |

- **`latent`.** The reference would be the posterior of a latent class model fitted with
  EM (Dawid-Skene). It is not available: `--reference latent` and `reference.mode =
  "latent"` are recognised and refused with "not available in this version", and `soft`
  stays the default. It does not ship in v1 (D-016): at three raters per item its
  intervals cover the true-class hit rate in fewer than half of simulated datasets. The check
  `latent_reference_unidentified` is registered but never fires, and per-rater sensitivity
  and specificity are not reported.

### 9.3 Rater design and the estimand

The overlap of raters and items decides the design, output as `rater_design`.

| Design | Detected when | Estimand and interval |
|---|---|---|
| `nested` (different raters per item) | the median rater labelled fewer than 20% of the items that have votes | performance against one rater drawn at random; resampling items already carries rater variation |
| `crossed` (the same few raters label many items) | otherwise | performance against these raters; items are resampled and raters are fixed |

- **Raters are not resampled.** Under nested designs, item resampling already contains
  rater variation and resampling raters would count it twice. Under crossed designs,
  resampling raters would treat a rater's systematic leniency as independent noise per item
  and not represent the population. With three raters of differing leniency, the standard
  error over new items and new raters is 0.056; resampling items gives 0.031, the error
  with these raters fixed, and adding raters per item or as a set gives 0.033 or 0.038,
  still 0.58 to 0.68 of it (`scripts/sim_rater_resampling.py`).
- **Rater spread (crossed, more than one voter).** The metrics are also computed with each
  rater's votes alone as the reference, and the minimum and maximum are output as
  `rater_spread` next to each metric.
- **Output.** `rater_design`, whether it was detected or set, the estimand sentence, the
  number of voters and the number of items per count of raters.
- `reference.rater_design` accepts `auto` (default), `nested` and `crossed`.

### 9.4 Does the system agree with people as well as people do (alternative annotator test)

The question is whether replacing one human rater with the system keeps agreement with the
remaining raters (Calderon, Reichart and Dror 2025; parameters in D-017).

- **Per rater.** For each human rater j, on the items j and at least one other human
  labelled and the system judged, the system and j are each scored by their agreement
  with the other humans on that item: the share of them with the same label for binary and
  nominal labels, minus the root-mean-square difference for ordered and score labels.
  W_sys = 1 when the system's score is at least j's, W_j = 1 when j's is at least the
  system's; a tie counts for both.
- **Test.** A one-sided one-sample t-test of d = W_j − W_sys with H0: mean(d) ≥ ε against
  mean(d) < ε, with n − 1 degrees of freedom. Rejecting H0 means the system wins against
  j, so the burden of proof is on the system and few items make winning harder. With no
  spread in d the mean decides alone.
- **Raters together.** Benjamini-Yekutieli over the tested raters at q = 0.05. The winning
  rate ω is the share of tested raters the system wins against: `human_parity` when
  ω ≥ 0.5, `below_human` otherwise. The advantage probability ρ, the mean over raters of
  mean(W_sys), is reported beside ω for choosing between systems.
- **Defaults and limits.** ε = 0.1, the paper's value for crowd raters and the strictest of
  its three (experts 0.2, skilled raters 0.15). A rater sharing fewer than 30 items with
  other humans is not tested and is listed as skipped; with fewer than two tested raters
  the verdict is `insufficient_data`. Three or more raters are recommended; with two, each
  is tested against the other.
- Reference case: `alt_test`, from scipy's `ttest_1samp` and `false_discovery_control`.
- **Where it runs.** `agree --parity --run <run>` tests that run against the human raters;
  the verdict is the envelope's, and the setting `agree.parity_epsilon` changes ε. A run
  with a decision rule is compared on decisions (positive when most of its replicates
  are), with the labels' positive definition; a run without one, on its values. A missing
  judgment that the missing policy counts as the worst agrees with no human (in a gate,
  every missing judgment). With two
  tested raters the output warns `two_human_raters`. In a gate, a `human_parity` condition
  with the rule `pass` runs the test on the holdout's items against the sealed human
  labels, with its `value` as ε (fixed at approval from `agree.parity_epsilon` when left
  out) and the raters adjusted together at the submission's level α/B instead of q =
  0.05, so the budget keeps its error rate; `insufficient_data` leaves the condition
  undecided.

### 9.5 Human reproducibility

- **Recording.** A rater's blind re-label of an item is a label with `kind = "independent"`
  and `label_replicate` of 1 or more. A correction of an earlier label is `kind =
  "revision"` with a higher `version`; analyses use the latest version. Both blind
  re-labels are kept.
- **Output (`agree`).** Per task, within-rater disagreement (pairs of one rater's replicates
  of an item) and between-rater disagreement (pairs of different raters' first labels of an
  item), each as pairs compared, pairs differing and their rate. If between-rater splits
  are as common as within-rater ones, the splits come from the items, not from the raters.
- **Adjudications** (`kind = "adjudication"`) were made after seeing others' labels, so they
  are not independent and enter neither agreement nor variability.

### 9.6 Split and unanimous items

The stratum `reference_agreement` puts items with two or more human votes into `unanimous`
and `split`. `report` grouped by this stratum gives the system's metrics for each.
Errors on split items are errors where people also disagree; errors on unanimous items
should weigh more. Items with fewer than two human votes have no stratum.

### 9.7 Meta-evaluation

A grader is treated as the system and the reference of §9.2 as the truth. `report` of the
grader's run gives §2 and §3, and `agree` with the run gives the agreement view:
κ, AC1 and the positive and negative agreement for two-class values; weighted κ and ρ for
ordered values; and the same-rule mean κ of §9.1.

### 9.8 Correcting a grader's bias (prediction-powered inference)

Most items have only a model grader's label f_i, and a random subset of n of the N items has
a human label y_i. The human-standard value θ is estimated by PPI++ (Angelopoulos et al.
2023), from `report` given a proxy: the model rater whose labels are f.

- **Metrics.** Accuracy of a decision run, and the mean human label for other runs
  (`mean_label`). Ratios whose denominator the label defines (H, F) are not covered, and
  item weights are not used.
- **Estimate.** θ̂ = λ·mean_N(f) + mean_n(y − λ·f), the first mean over all items.
- **Variance.** λ² Var(f)/N + Var(y − λf)/n, with a normal interval.
- **λ.** λ* = Cov(y, f) / ((1 + n/N) Var(f)), clipped to [0, 1]; λ = 0 uses human labels
  alone and λ = 1 is the original PPI. It needs at least two items in each group.
- **Labels that vary.** y_i is the reference of §9.2 (s_i under `soft`), so people's
  disagreement is in Var(y − λf).
- **Condition.** The human-labelled items must be a random sample. evalsig accepts that only
  when every item states the same `label_inclusion_prob`; otherwise λ = 0, the estimate
  uses human labels alone, and the output says `corrected: false`.
- Reference case: `ppi_mean`.

### 9.9 Labels on only some items (verification bias)

- **The problem.** When people check only items the system called positive, raw H and F are
  biased (partial verification bias).
- **Correction.** When an item states `label_inclusion_prob` p, its weight is multiplied by
  1/p (inverse probability weighting, in the spirit of Begg and Greenes 1983). Every ratio
  metric then uses the weights, and the interval is a bootstrap.
- **Without probabilities.** The check `verification_bias_suspected` fires when some judged
  item has no human label and no probability, and the share of human-labelled items differs
  by at least 0.2 between two judgment levels that each hold at least 10 items.

### 9.10 Items needed to measure human variation

The number of items two raters both label follows from the half-width wanted for a
disagreement rate: `docs/sample-size.md`, table 2 (a 10% to 20% rate to ±0.05 takes 141 to
245 items). Not every item needs two raters: a random subset (say 200 items) labelled
twice, the rest once, is enough, with the estimand stated by the design of §9.3.

## 10. Pairwise preference

A `preference` record compares two arms' outputs on one item and says which is better.

- **Score.** A judgment of arms A and B is scored for B: 1 when B wins, 0 when A wins,
  ½ for a tie. A preference with no outcome is left out.
- **Win rate.** The mean score of an (item, rater) group, averaged over the item's raters,
  then over items. If a group holds both presentation orders, its mean cancels position.
  The interval is a bootstrap over items (at least 5 items). All raters with a judgment of
  the two arms count; the output lists them and their `source`s.
- **Position bias.** Over (item, rater) groups judged in both presentation orders (swapped
  pairs): the share whose two orders agree (order consistency), and the share of decided
  (non-tie) judgments won by the left side with its Wilson interval. If that interval
  excludes ½, the warning `position_bias` fires. With no swapped pair, `no_swapped_pairs`
  fires.
- **Rater agreement** (`agree` on preferences). For each pair of raters with a shared item,
  κ and AC1 over the order-normalised values A, B and tie (a fixed 3×3 table), by §9.1's
  definitions, and each rater's position bias.
- **Three or more arms.** A Bradley-Terry model, with Davidson's tie parameter when ties
  occur, is fitted by maximum likelihood (Newton's method). The output ranks the arms by
  log-strength, centred to mean 0, with percentile bootstrap intervals. If the comparison
  graph of the arms is not connected, the warning `comparison_graph_disconnected` fires and
  no ranking is given; an arm that wins or loses every comparison has no finite strength.
- Reference case: `bradley_terry`.

## 11. Error control of the gate

### 11.1 Statistical rules

- **Rules.** A condition reads an interval or a verdict, never a point alone: `lower_bound_at_least`
  and `upper_bound_at_most` read the candidate's interval; `delta_lower_bound_at_least`,
  `delta_upper_bound_at_most` and `not_worse_than` (non-inferiority, a negative margin) read
  the interval of the difference, candidate minus baseline; `pass` reads a verdict, which only
  `human_parity` gives (§9.4). The two
  delta rules also need the point difference to reach `min_effect` in their direction. A
  condition whose evidence is missing or of the wrong kind is undecided.
- **Several conditions.** All must hold: an intersection-union test (Berger 1982) keeps the
  overall error rate within the level of each condition, so no further correction is
  needed. One failed condition fails the gate; otherwise an undecided one leaves it
  undecided (`insufficient_data`).
- **Repeated submissions.** With a budget of B submissions, each is judged at α/B
  (Bonferroni). A two-sided interval is taken at 1 − α/B, a one-sided bound at 1 − α/B
  through the two-sided interval at 1 − 2α/B.
  - For example, B = 20 submissions each judged at α = 0.05 pass an ineffective candidate
    at least once with probability 1 − 0.95²⁰ ≈ 64% if the tests are independent, and up
    to 100% if they are dependent.
  - **The budget belongs to the sealed holdout.** B counts submissions per sealed holdout
    (its seal hash). Rewriting and re-approving the conditions does not reset it; a new
    budget needs a new split and seal. The default is 5.
- **Direction.** Each condition names `sided`; improvement and non-inferiority conditions
  are usually one-sided.
- **Counts.** `min_items` is required and counts only paired items, independent of the
  labels. Judging by the number of positives or negatives would let labels be inferred from
  where "too few items" begins. Fewer items gives `insufficient_data`.
- **Power at approval.** When the first condition is a delta rule with `min_effect` above
  0, `preregister approve` computes the probability that it passes if the candidate is
  better by exactly `min_effect`, on the holdout's items at the per-submission level, and
  warns (`low_power_at_hypothesis`) below 0.5. It uses the one-sided exact McNemar power
  (§12) with the discordance ψ̂ = max(0.05, 4/n, min(1, 2·e_val)), n the holdout size and
  e_val the baseline's error (1 − accuracy) on the items outside the holdout. No power is
  computed when `min_effect` is at least ψ̂.
- **What the guarantee rests on.** An ineffective candidate passes within the budget with
  probability at most α only if each test holds its nominal level (§1.1, §8) and the
  candidate's records pass the checks of §11.2. Choosing the next candidate from what the
  gate returns loosens the bound; §11.3 limits what it returns.
- **Seed.** A gate's resampling seed is derived from the approval and the candidate's
  snapshot hash, so the same submission gives the same answer.
- Reference case: `gate_rules`.

### 11.2 Checks the referee makes on the records

On the holdout the candidate's records are a claim to check. The referee refuses or stops
as follows; no override is accepted.

| Check | Behaviour |
|---|---|
| the candidate's records cover every holdout item | otherwise refused with exit 3 before any label is read; no budget is spent. No absent item is scored |
| the same snapshot of the same candidate submitted again | the earlier answer is returned and no budget is spent |
| `run.grader` id, version and `config_hash` match the approval | otherwise exit 3. It stops mix-ups, not an altered grader |
| the candidate's output type, levels and value semantics match the approval | otherwise exit 3 |
| the candidate was made with the approved mapping | otherwise exit 3 |
| generation replicates: fewer than the approved number | exit 3 |
| every generation replicate of some item has the same output (`identical_replicates`) | stopped (exit 5) |
| a holdout item whose reference labels all come from models | stopped (exit 5) |
| fewer paired items than `min_items` | exit 4, no decision |
| the budget is spent | exit 2 |
| every missing judgment, whatever its reason | counts as worst |

After coverage passes, every outcome spends one submission. The answer carries the
verdict, the pass or fail of each condition and the budget left; it carries no interval,
value or item result.

### 11.3 Returning less information (Ladder)

The Ladder is a scheme that returns a value only when a candidate beats the best so far by
a step η, and rounds the value to a multiple of η toward the baseline (D-015). Approval
fixes the step and the first best on the primary condition's metric (the first
condition): η = max(k·√(ψ̂/n), 1/n) with k = `ladder_k` (2 by default, at least 1), n the
metric's denominator on the holdout and ψ̂ = max(0.05, 4/n, min(1, 2·e_val)), e_val the
baseline's error outside the holdout; e_val = 0.1 and n = 300 give η ≈ 0.052. These are
read with every missing value the worst, as the gate reads the holdout. A `ladder_step`
below the automatic η is a usage error, since a finer step would return detail below the
standard error of the difference. The first best is the baseline on the holdout,
which spends no budget. Without a baseline, or for a metric with no value on both sides of
the holdout, there is no step, approval warns `ladder_unavailable`, and no value is ever
returned.

- **A new best.** A submission beats the best so far when the lower bound of its paired
  difference to the best on the primary metric, one-sided at α/B, is above 0 and the
  difference is at least η. The best is the baseline until a candidate beats it; the
  referee then keeps that candidate's run and judgments, private to it, as the best of
  these approved conditions, so other conditions or a re-approval start from the
  baseline again.
- **What comes back.** On a new best only, `results.ladder` holds the metric, η and the
  value rounded to a multiple of η toward the baseline: the candidate's value for an
  absolute rule, its difference to the baseline (rounded toward 0) for a difference rule.
  The referee ledger's `gate` line records the value. Otherwise the answer carries no
  value, only the verdict, each condition's pass or fail and the budget left (§11.2).
- **How much one answer reveals.** At most 1 bit for the verdict, 1 bit per condition
  and, on a new best only, about log2(1/η) bits for the rounded value (about 4 bits at
  η ≈ 0.05). The gate returns nothing beyond this.

The protection against fitting the holdout rests on the budget, the level
α/B and withholding values; at a few hundred holdout items the Ladder's own theoretical
guarantee, which needs steps near (ln B / n)^(1/3), would hardly apply (D-015).

## 12. Power and sample size

`evalsig power` computes these; the tables are in `docs/sample-size.md`, generated by
`scripts/sample_size.py` and checked against `evalsig_core::power`.

- **Claiming a rate below u with no events.** n = ⌈ln α / ln(1 − u)⌉ (exact, §1); the
  Wilson count ⌈z²(1 − u)/u⌉ is shown beside it.
- **Measuring a rate to ±w.** The smallest n whose two-sided Wilson interval at rate p has
  half-width at most w (above 200 items the search steps by n/200).
- **Detecting a paired difference.** The power of the exact McNemar test is summed
  exactly: the number of discordant pairs D ~ Binomial(n, ψ), and given D, b ~ Binomial(D,
  p_10/ψ). Power is not monotone in n, so the answer is the smallest n from which power
  stays at or above the target (the first n of a run of 20 evaluated points that all
  reach it; the search steps by 1 below 400 items and by 5 above, and gives up at 20 000).
  Connor's approximation (1987), ⌈(z_α √ψ + z_β √(ψ − δ²))² / δ²⌉, is shown beside it, and
  the minimum detectable difference is found from it by bisection.
- **Clusters.** The items needed are multiplied by the design effect 1 + (m̄ − 1)ρ for mean
  cluster size m̄ and intra-cluster correlation ρ.
- **A gate's level.** Given a gate's conditions, each submission is judged at α/B, and
  `power` reports the needed items and the pass probability with the given items when the
  hypothesised effect is real. For example, a difference of 5 points with 10%
  discordance, power 0.8 and a one-sided α = 0.05 needs 246 items (Connor), and
  α = 0.05/20 needs 530.
