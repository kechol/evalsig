# Changelog

Each release has a section headed `## [<version>] - <date>`; the release
workflow refuses a tag whose version has no such section, or one still marked
Unreleased (`scripts/check_changelog.sh`).
Changes not yet released go under `## [Unreleased]`.

Within a section, list changes under Added, Changed, Fixed and Removed. A change
to how a metric is defined — its formula, method or defaults — is listed under
**Metric definitions**, together with the new `spec_version` that outputs carry
in `provenance.spec_version`, so `evalsig verify` can tell why a saved result no
longer reproduces.

## [Unreleased]

### Changed

- Dashboard: the overview writes each metric's value and interval beside its plot on
  an axis shared by all runs and links each run to its comparison and to the screens
  about one run; links between screens keep the chosen run or pair; the comparison
  lists the items whose judgment changed in a table of their own, opens on the latest
  comparable pair and keeps the choice on the page when two runs cannot be compared;
  numbers are written with four decimals; screens with nothing to show say which data
  they need.
- Dashboard and `--format html`: one stylesheet of tokens and components, in light and
  dark, with screens as tabs, sections as panels and verdicts coloured by outcome.
- Dashboard charts (D-032): the equal-variance model of d′ and c, runs in ROC space
  (both runs of a comparison), same-rule agreement as bars and performance by k as
  lines with interval bands; ROC, z-ROC and calibration curves gain ticks, z-ROC in z
  units; replicate agreement is a table of its measures.
- Dashboard in Japanese (D-033): `[dashboard] language = "ja"` writes the pages' own
  text in Japanese; verdicts, warnings and glossary meanings stay English.
- Dashboard design tokens retuned to neutral greys, underlined tabs and lighter tables.
- `trend` without `occurred_at` hints at adding it.

- Gate (D-034): the difference rules are `better_by` and `not_worse_than`, both on the
  difference oriented so that positive is better, both strict, with margins of at least
  0; `delta_lower_bound_at_least` and `delta_upper_bound_at_most` are gone. A difference
  condition may state `expected_effect`, and approval reports each such condition's
  pass probability (`power`) and warns below 0.8 (`low_power`).

### Fixed

- Exact bounds on millions of judgments no longer hang: above a total shape of 1e5 the
  Beta quantile bisects the distribution function (D-036).

- `compare` (D-035): each run's item inputs come from its own imports, so items whose
  `input_ref` changed between the arms are left out and reported as `input_changed`;
  before, both arms read the latest import's item records and the check never fired.

- `compare` and `gate`: a score run read at a threshold (`rate_at_least`) takes Tango's
  interval and the exact McNemar test like the other paired rates, not the bootstrap.
- `compare`, `report` and `gate`: a bootstrap interval needs five units that carry the
  metric's denominator, so positives in a few clusters no longer give the hit rate a
  zero-width interval or a gate pass.
- `gate`: `not_worse_than` reads the difference oriented so that positive is better, so
  its margin bounds the loss on `false_alarm_rate`; `missing_rate` counts items, not
  judgments; a condition without `level` takes the configured level, not the first
  condition's; approval reaches the percentile tail of each condition when it fixes the
  resamples, keeps a holdout's budget, and reads the baseline with the draft's reference
  mode; a run that mixes output types, or has fewer replicates than k of an @k
  estimand, is refused; a refusal after the budget is spent is replayed when the same
  snapshot comes again.
- `compare` and `report`: the transition table and the PPI accuracy fill a system-side
  gap as accuracy does; missing bounds keep the gaps a policy drops;
  `n_insufficient_replicates` counts the replicates left after the policy drops some;
  the Brier score and reliability bins are weighted; d′, c and the feature likelihood
  ratios read their counts on the scale of effective items; the optimal threshold takes
  the lower level on an exact tie; a majority@k value never exceeds 1.
- `agree`: ordered labels that are not integers get Spearman's ρ; AC1 and the specific
  agreements stay defined for a pair that uses one class; the weighted κ no longer
  allocates a table over the whole label range.
- `power`: out-of-range settings are usage errors, `--half-width 0` no longer loops, the
  exact power at ψ = 1 is finite, and `--gate` reads the level an approval resolved.
- `import --mapping`: an integral float such as `3.0` is the ordinal level it spells.
- The dashboard shows the validity-check findings and blocked verdicts the command line
  shows; the Markdown report and the verdict text keep the decimals of the interval level;
  a decimal split share is not rounded up by floating-point error; the ROC sweep is
  linear in the number of distinct scores, and a NaN score no longer stops it.

## [0.1.0] - 2026-11-07

### Added

- First release: the evalsig command-line tool (import, report, compare, trend,
  agree, power, check, the referee commands and gate serve), the agent interface
  and the local dashboard.

### Metric definitions

- `metrics/1`: the definitions of the metrics specification at this release.
