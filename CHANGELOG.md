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

### Added

- Errors list the commands to run next in `error.next` (D-040), built from evalsig's
  own vocabulary so agent mode prints them in full.
- A bare `evalsig` in agent mode or with `--format json` prints the command index
  (`evalsig.index/1`) and exits 0.
- `exclude --dry-run` and `split --dry-run` report what they would record without
  writing anything.
- `compare` compares expected cost (`expected_cost`) when `[cost]` is set and the runs'
  judgments are levels, with a paired cluster-bootstrap interval and a verdict (D-041).
  `[cost] by` and `[cost.strata."<value>"]` set a prevalence, a matrix and a production
  share per stratum; the comparison then has one row per stratum. `report` gives the run's
  expected cost overall and per stratum with cluster-bootstrap intervals, and the
  dashboard's discrimination screen draws it.
- Input files compressed with gzip or Zstandard are read as their content, within the
  size limit, by `import`, `validate --records`, `gate submit` and the `map` commands
  (D-046).
- An `attribution` record names the causes a rater gave to a run's error on an item
  (D-048); `import`, `validate --records`, `export` and the Python and TypeScript writers
  handle it.
- A `run` record may declare per feature the class it should point to (`expected`) and a
  `lineage` kept across renames and splits (D-049). `report --features` then says whether
  each declared feature points the way it should (`direction`).
- `report --decompose misses|false_alarms` splits a run's errors by attributed cause, each
  error to its first cause in `[attribution] order`, with shares and `--ceiling` rates and
  their intervals (D-048).
- `label.join.key` joins a JSONL or CSV label file by a key built from each label row and
  each source row, such as `{@.task}:{@.unit_id}`; every matching row gives a label (D-047).
- `judgment.explode.filter` in a mapping keeps the elements or keys a condition accepts,
  such as replicates a log marks as copies (D-046).
- `compare --sdt` adds ΔH, ΔF, Δd′ and Δc with paired intervals and classifies the change
  as `discrimination_improved`, `discrimination_worsened`, `threshold_moved` or
  `undetermined` (`insufficient_data` below `--min-events`, default 5) (D-042).

### Metric definitions

- `metrics/2`: `compare` compares expected cost (metrics §4, §7) and, with `--sdt`, d′ and
  c (§7); expected cost takes
  per-stratum prevalences, matrices and production shares (D-041). Saved comparisons of a
  project with `[cost]` set gain an `expected_cost` row.

### Changed

- Common usage errors have codes of their own instead of `usage` (D-040):
  `unknown_command`, `missing_command`, `unknown_argument`, `missing_argument`,
  `invalid_value`, `conflicting_arguments`, `unknown_run`, `unknown_item`,
  `unknown_split`, `unknown_metric`, `unknown_period`, `unknown_name`,
  `already_exists` and `confirmation_required`. The exit code stays 64.
- A parse error's message names what is missing or misspelt: it keeps every
  paragraph of the parser's message except the usage line.
- Hints name commands without backticks, so agent mode no longer withholds them.

## [0.1.0] - 2026-11-09

### Added

- First release: the evalsig command-line tool (import, report, compare, trend,
  agree, power, check, split, verify, the referee commands and gate serve), the agent
  interface and the local dashboard, in light and dark and in English or Japanese
  (`[dashboard] language = "ja"`).

### Metric definitions

- `metrics/1`: the definitions of the metrics specification at this release.
