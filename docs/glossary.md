# Glossary

The specification, CLI output, reports and the dashboard use only the terms in
this file. Each term gives the identifier used in CLI flags and JSON keys.

## Rules for this file

- Add a term here before using it in user-facing output or documentation.
- Prefer the established term from statistics, signal detection theory or
  machine learning. When two names are common, list the second in the
  meaning.
- **Split signal**: when this file exceeds 1,000 lines or 300 terms, report it
  and follow "Splitting a ledger" in `AGENTS.md`.

## Data

| Term | Identifier | Meaning |
|---|---|---|
| item | `item` | One object being judged. An Inspect sample or a promptfoo test case |
| stratum | `stratum` | A grouping of items to report separately (language, difficulty, customer). An item may have several |
| weight | `weight` | Weight that maps the item back to the population. Default 1 |
| reference label | `label` | A judgment treated as correct. Its `source` says whether a human, a model or a panel produced it |
| rater | `rater` | The human or model that produced a reference label. Also called annotator |
| system under test | `system` | What produces judgments (a model, prompt and rule combination). Has a version |
| judgment | `judgment` | One output of the system under test for one item |
| output type | `output_type` | `binary` (two values), `ordinal` (k ordered levels) or `score` (continuous) |
| generation replicate | `gen_replicate` | Which re-run of the system on the same item. Inspect's epoch, promptfoo's repeat |
| grading replicate | `grade_replicate` | Which re-grading of the same output by a grader |
| cluster | `cluster` | A set of correlated items (same document, same conversation, several tasks on one object). The resampling and splitting unit |
| value semantics | `value_semantics` | `decision` (a system decision compared with reference labels) or `score` (a grade of an output that is itself the result; no reference label needed) |
| decision rule | `decision` | From which judgment value a judgment counts as positive, e.g. `score_at_least = 0.5`. Separate from the label-side `positive` |
| label inclusion probability | `label_inclusion_prob` | When humans check only some items, the probability that this item received a human label |
| preference | `preference` | A record comparing the outputs of two arms on the same item and saying which is better |
| feature | `feature` | A rule, aspect or assertion that fired inside a judgment, with whether it fired and its strength |
| content | `content` | Text such as the input, the output and a grader's reasoning |
| run | `run` | One import unit: the judgments of one system version on one dataset |
| arm | `arm` | One side of a comparison, such as "before" and "after" |
| missing | `missing` | An item whose judgment or label could not be obtained. Carries a reason code and is never silently dropped from the denominator |
| mapping | `mapping` / `mapping.toml` | A declaration that maps external records to the canonical format (row splitting, ID assembly, value mapping, missing conditions, joining labels from another file). Its hash is kept as provenance |
| decisions needed | `decisions_needed` | Mapping choices that change results and are never guessed. `map decide` records the human answer |
| capabilities | `capabilities` | The analyses the current data supports, the ones it does not, and how to obtain the missing information |
| migration | `migrate` | Raising the version of evalsig's own canonical format. Distinct from mapping external records (`map`) |

## Signal detection and classification

| Term | Identifier | Meaning |
|---|---|---|
| positive / negative | `positive` | Whether the reference label is a hit or a miss. Which value is positive is configured |
| prevalence | `prevalence` | The share of positives in the population. Distinct from the share in the test set. Also called base rate |
| hit rate | `hit_rate` (H) | Share of positives the system judged positive. Same as sensitivity, recall and true positive rate |
| false alarm rate | `false_alarm_rate` (F) | Share of negatives the system judged positive. Same as false positive rate |
| false negative / false positive | — | A missed positive / a negative judged positive |
| precision | `precision` | Share of positive judgments that were actually positive. Depends on prevalence |
| sensitivity index | `dprime` (d′) | How well positives and negatives are told apart. d′ = z(H) − z(F) |
| criterion | `criterion` (c) | Where the system starts saying "positive". c = −(z(H) + z(F))/2. Negative means liberal. Distinct from gate `conditions` |
| calibration | `calibration` | How well scores read as probabilities match outcomes. Brier score and reliability diagram |
| ROC curve | `roc` | The (F, H) path as the decision threshold moves |
| AUC | `auc` | Area under the ROC curve. The probability that a random positive is scored above a random negative |
| z-ROC | `zroc` | ROC with both axes transformed to normal quantiles. The straighter, the better the equal-variance normal model fits |
| d_a | `d_a` | Sensitivity index when the z-ROC slope is not 1 (unequal variances) |
| isosensitivity curve | — | The (F, H) curve of constant d′. Moving along it only shifts the criterion |
| likelihood ratio | `likelihood_ratio` (LR), `features` | Feature firing probability among positives divided by that among negatives. 1 means uninformative. `features` lists each feature's LR and class |
| decision threshold | `threshold` | The score or level from which a judgment counts as positive |
| cost matrix | `cost_matrix` | Loss per item for each (reference label, judgment level) cell |
| expected cost | `expected_cost` | Average loss per item from the cost matrix and the judgment distribution |

## Agreement and repeatability

| Term | Identifier | Meaning |
|---|---|---|
| replicate disagreement | `replicate_disagreement` | Share of items whose replicates disagree. A repeatability measure |
| noise floor | `noise_floor` | How much a metric moves when the same configuration is simply re-run. Included in the intervals and shown as a diagnostic, not used separately in verdicts |
| reference mode | `reference.mode` | How several human labels become a reference: `single` (one rater or adjudication), `majority`, `soft` (share of raters saying positive) or `latent` (Dawid-Skene; needs three or more raters per item) |
| rater design | `rater_design` | `nested` (different raters per item) or `crossed` (the same few raters label many items). Changes the estimand. Raters are not resampled |
| reference agreement | `reference_agreement` | Stratum of items on which all human raters agreed (`unanimous`) or not (`split`) |
| ceiling | `ceiling` | With a soft reference, the accuracy no system can exceed: mean(max(s, 1 − s)) |
| same-rule comparison | `same_rule` | Human–human and system–human agreement measured the same way: each rater, grader or run against each human rater but itself, averaged (leave-one-out) |
| rater variability | `within_rater`, `between_raters` | Disagreement among one rater's own label replicates of an item, and among different raters' labels of it |
| label kind | `label.kind` | `independent` (made without seeing others), `revision` (a rater correcting their earlier label) or `adjudication` (resolving a split; not independent) |
| label replicate | `label_replicate` | A blind re-label of the same item by the same rater. Measures human variability |
| alternative annotator test | `alt_test` | Tests whether replacing one human rater with the system keeps agreement with the remaining raters |
| Gwet's AC1 | `gwet_ac1` | An agreement coefficient that does not collapse under skewed prevalence |
| positive / negative agreement | `positive_agreement` / `negative_agreement` | How well raters agree on positive judgments and on negative judgments separately |
| k-run rules | `majority@k`, `any@k`, `all@k` | Performance when k generations are combined by majority, any or all. `any@k` is pass@k for generation tasks |
| inter-rater agreement | — | Agreement between raters beyond chance: Cohen's κ, quadratic-weighted κ, Fleiss' κ, Krippendorff's α, Spearman's ρ. System–human agreement uses the same rule (pairwise against each rater) |
| grader | `grader` | What scores a judgment: an LLM judge, a rule or a human. Also called judge |
| meta-evaluation | — | Measuring a grader itself against human reference labels |

## Comparison and inference

| Term | Identifier | Meaning |
|---|---|---|
| confidence interval | `ci` | The uncertainty range of an estimate. 95% by default |
| one-sided upper bound | `upper_bound` | A value the rate is at most, at the stated level. Used by gates |
| paired comparison | — | Comparing two arms on the same items. McNemar's test, paired bootstrap |
| estimand | `estimand` | Which performance is estimated. Default `single_shot` (one production run); also `majority@k`, `any@k`, `all@k` |
| mean score / rate at least | `mean_score`, `rate_at_least` | Metrics usable without reference labels: the mean judgment value and the share at or above t |
| unpaired comparison | — | Comparing two sets of items that do not overlap (Newcombe interval, Welch's t) |
| trend | `trend` | Per-period metrics with intervals. Periods outside the baseline are `drifted` |
| winner's curse | — | Picking the best of many candidates biases its estimate upwards |
| minimum effect | `min_effect` | Differences smaller than this are not called improvements even if the interval excludes 0 |
| missing bounds | `missing_bounds` | The metric range when system-side missing judgments are filled with the best and worst values (Manski bounds) |
| position bias | — | In pairwise comparison, the side shown first (left) tends to win |
| prediction-powered inference | `ppi`, `proxy` | Combines many grader labels with a few human labels to estimate the human-standard value without bias. `proxy` names the model rater whose labels fill in; `corrected` is false when the human-labelled items are not known to be random, and only the human labels are used |
| verification bias | — | Bias in hit and false alarm rates when humans check only some items |
| confidence sequence | `confidence_sequence` | An interval that stays valid however often it is inspected as data accrue |
| validation uses | `validation_uses` | How many comparisons used the same validation split. Above the configured limit, evalsig warns that it is time to rebuild the split. No correction is applied |
| transition table | `transition` | How many items moved correct→wrong and wrong→correct between two arms |
| power | `power` | The probability of detecting a real difference |
| minimum detectable effect | `mde` | The smallest difference detectable with the given size and power |
| multiplicity correction | — | Controlling false positives when testing many metrics, strata or candidates at once (Holm) |
| regression to the mean | — | Picking what looked bad and fixing it makes it look better next time even without a real change |
| effective sample size | `n_eff` | The loss of sample size under weights. Kish: (Σw)²/Σw² |

## Experiment procedure and audit

| Term | Identifier | Meaning |
|---|---|---|
| dev / validation / holdout | `dev`, `validation`, `holdout` | Splits for looking at while fixing, for deciding adoption, and for the final check |
| split | `split`, `split_part` | An assignment of whole clusters to dev and validation, named by the hash of each part's item set. `split_part` names the part a comparison used |
| remaining items | `remaining.json`, `holdout_excluded` | The item IDs the referee leaves to the agent after cutting the holdout; a split made within them has `holdout_excluded` true |
| seal | `seal` | Encrypting holdout reference labels so only the referee holding the key can use them in `gate`. The labels never enter the agent's environment |
| conditions | `conditions` | The rules a gate requires. Pass means all hold |
| check | `check` | An absolute-threshold check with intervals, without preregistration or holdout |
| threat model | — | What the referee protects and what it does not (specification §8.0) |
| adaptive overfitting | — | Fitting the holdout by trying repeatedly while looking at results |
| holdout budget | `budget` | The maximum number of queries to a holdout. Counted per sealed holdout in the referee ledger |
| preregistration | `preregister` | Recording gate conditions and hypotheses in the ledger before seeing results. Two steps: a draft (`--draft`, by an agent or a human) and approval (`approve`, referee authority only) |
| gate | `gate` | Deciding pass or fail under preregistered conditions and returning it as the exit code |
| snapshot | `snapshot` | An immutable copy of imported data. Identified by its content hash |
| ledger | `ledger` | An append-only record of imports, revisions, splits, gates and other operations. Each line carries the hash of the previous one. There is a referee ledger and an agent-side ledger; only the referee ledger is evidence |
| provenance | `provenance` | Where data and results came from, with which version and configuration |
| content | `content` / `store.content` | The text of an input, output or grader rationale, stored as an object and named by records through its hash. Kept in full, as a hash only, or with `redact_patterns` matches replaced by `[redacted]` |
| migration | `migrate` | Rewriting snapshots in an earlier canonical record format as new snapshots in the current one, with a ledger line; the originals are kept |
| purge | `purge` | Removing a content body and the free-text fields of the items that name it, through a replacement snapshot and a ledger line; the superseded snapshot remains only as a hash |
| validity checks | `checks` | Checks that stop measurement errors: missing denominators, mixed versions, double-counted replicates |

## Agent interface

| Term | Identifier | Meaning |
|---|---|---|
| verdict | `verdict` | The code that states an output's conclusion (`improved`, `negligible`, `indistinguishable`, `worsened`, `drifted`, `insufficient_data`, …) with a sentence. Determined only by formulas and configuration |
| agent mode | `agent_mode` | Output for AI agents: a JSON envelope with a size limit that omits content by default |
| envelope | `envelope` | The JSON shape shared by all commands (`verdict`, `results`, `warnings`, `next`, `provenance`) |
| untrusted content | `untrusted_content` | Text under evaluation. Passed as data whose instructions are not followed |
| referee | `gate serve` | A process or CI job, under a different authority from the agent, that holds the private key, the holdout labels and its own ledger, and performs split, seal, approval and gating |
