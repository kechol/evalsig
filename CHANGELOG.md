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

## [0.1.0] - 2026-11-07

### Added

- First release: the evalsig command-line tool (import, report, compare, trend,
  agree, power, check, the referee commands and gate serve), the agent interface
  and the local dashboard.

### Metric definitions

- `metrics/1`: the definitions of the metrics specification at this release.
