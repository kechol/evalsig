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
