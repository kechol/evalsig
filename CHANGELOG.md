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
- `trend` without `occurred_at` hints at adding it.

## [0.1.0] - 2026-11-07

### Added

- First release: the evalsig command-line tool (import, report, compare, trend,
  agree, power, check, the referee commands and gate serve), the agent interface
  and the local dashboard.

### Metric definitions

- `metrics/1`: the definitions of the metrics specification at this release.
