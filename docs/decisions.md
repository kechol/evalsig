# Decisions

This ledger records choices about architecture, dependencies, the data model,
public APIs and authentication. Code that depends on a decision lands only
after the entry is `Accepted`.

## Rules for this file

- IDs (`D-NNN`) are permanent. Never renumber; append the next number after
  the highest in use.
- Status is one of `Proposed`, `Accepted` or `Superseded by D-NNN`. To change
  an accepted decision, add a new entry with a `Supersedes: D-NNN` line and
  set the old entry's status; do not rewrite the old entry's reasoning.
- Each entry has: Status, Date, Context, Decision, Alternatives and
  Consequences. Keep it to what a future reader needs to understand why.
- The commit that settles an entry carries `Decision: D-NNN`. If no code
  depends on it yet, the commit may be empty (`git commit --allow-empty`).
- **Split signal**: when this file exceeds 1,500 lines or 60 entries, report
  it and follow "Splitting a ledger" in `AGENTS.md`.

## D-001 Workspace layout, toolchain and baseline dependencies

- Status: Accepted
- Date: 2026-11-03
- Context: evalsig needs a pure, separately publishable statistics library and
  several I/O-heavy front ends. Most tasks need the same few general-purpose
  crates, and asking for a decision each time would add no information.
- Decision:
  - A Cargo workspace under `crates/` with five members:
    `evalsig-core` (types, metrics, validity checks; no I/O),
    `evalsig-store` (objects, ledgers, SQLite, canonicalization, hashing,
    sealing), `evalsig-adapters` (ingestion), `evalsig-cli` (binary
    `evalsig`) and `evalsig-dashboard` (web server, SVG).
  - Dependencies point inwards: `core` depends on no other member; `store`
    and `adapters` may depend on `core`; `cli` and `dashboard` may depend on
    everything below them.
  - Rust edition 2024. The development toolchain is pinned in
    `rust-toolchain.toml`; the MSRV is declared as `rust-version` in the
    workspace manifest and built in CI.
  - Baseline dependencies that any member may use without a further decision:
    `serde` (with `derive`), `serde_json`, `toml`, `thiserror` for library
    errors, and in the binary crates `clap` (with `derive`) and `anyhow`.
    Test-only crates (`proptest`, `assert_cmd`, `predicates`, `tempfile`) need
    no decision either. Every other runtime dependency goes through this file.
- Alternatives: a single crate (rejected: `evalsig-core` must be publishable
  without the CLI, SQLite and web stacks); more, finer crates (rejected: no
  consumer needs the extra boundaries yet).
- Consequences: library crates return typed `thiserror` errors; only the
  binaries use `anyhow`. Raising the MSRV is a release-note item.

## D-002 Licence and documentation language

- Status: Accepted
- Date: 2026-11-03
- Context: evalsig is a personal open-source project meant to be embedded in
  other Rust tools and read by an international audience.
- Decision:
  - Every crate is licensed `MIT OR Apache-2.0`, with `LICENSE-MIT` and
    `LICENSE-APACHE` at the repository root. Contributions are accepted under
    the same terms.
  - English is the canonical language of all documentation, code comments,
    CLI output and commit messages. In v1, only `README` and the quick start
    also exist in Japanese (`README.ja.md`, `docs/quickstart.ja.md`); the
    English version wins where they differ.
- Alternatives: MIT only or Apache-2.0 only (rejected: the dual licence is the
  Rust ecosystem norm and lets users pick); translating all documents
  (rejected: two canonical copies drift).
- Consequences: dependencies must have licences compatible with
  `MIT OR Apache-2.0`; `cargo deny` enforces the allow list.

## D-003 Canonical record format `records/1`

- Status: Accepted
- Date: 2026-11-03
- Context: Every import path (custom harnesses, declarative mappings, Inspect,
  promptfoo) must end in one format that analysis, hashing and the referee
  share. Its shape is a public contract: users write it directly.
- Decision:
  - `records/1` is JSONL with one record per line, tagged by `record`:
    `item`, `label`, `judgment`, `preference`, `run` and `content`, with the
    keys and fields of `schema/records.schema.json`. Unknown records and
    unknown fields are rejected.
  - A judgment, label or preference carries exactly one of `value`
    (`outcome` for preferences) and `missing{reason, detail?}`. The reason is
    one of ten codes: `error`, `timeout`, `refused`, `invalid_output`,
    `limit` (system side) and `grader_failed`, `not_run`, `invalidated`,
    `excluded`, `other` (measurement side).
  - `value` is a boolean for `binary`, an integer in `0..levels` for
    `ordinal` (larger is more positive) and a finite number for `score`.
    Each run has one `value_semantics` (`decision` by default, or `score`,
    which requires `grader`) and one `output_type` with `levels`.
  - Labels are never overwritten: a revision of the same (item, rater, task,
    label replicate) appends a higher `version`.
  - Absent optional fields stay absent when a record is written back; the
    schema defaults (weight 1, task `default`, replicate 0, kind
    `independent`) are applied when a record is read for analysis.
  - The schema `$id` is `urn:evalsig:schema:records:1`, so the identifier does
    not depend on a domain.
- Alternatives: one wide table with nullable columns (rejected: preferences
  and content do not fit, and the per-kind keys would be implicit); a value
  field with a sentinel for missing (rejected: it hides missing data in the
  denominator, which the specification forbids).
- Consequences: changing a field or a reason code is a new format version
  (`records/2`) with a `migrate` path. Integer and floating-point values are
  kept apart so records serialize back as written.

## D-004 Project configuration file

- Status: Accepted
- Date: 2026-11-03
- Context: The positive definition, decision rules, missing-value handling,
  store mode, import blocks, seeds and interval method change results, so
  the file that holds them is part of every result's provenance.
- Decision:
  - The project configuration is TOML at `.evalsig/config.toml`, with the
    tables `[project]` (name, `positive`, `decision`, per-task
    `[project.tasks.<task>]` overrides), `[store]`, `[stats]`, `[missing]`,
    `[reference]`, `[compare]`, `[cost]`, `[checks]` and `[[import]]`, as in
    `examples/evalsig.toml`.
  - Rules are one-key tables: `{ ordinal_at_least = n }`,
    `{ score_at_least = x }` or `{ equals = v }`. With no `positive`, binary
    labels treat `true` as positive; other label types need one.
  - Every table rejects unknown keys and suggests the closest known key.
  - For hashing, the configuration is converted to JSON with every default
    filled in, then canonicalized like any other object, so files that mean
    the same thing hash the same.
  - The referee never reads this file. Values that affect a gate are resolved
    into the approved preregistration.
- Alternatives: YAML (rejected: implicit typing turns `no` and `1.10` into
  other values); ignoring unknown keys (rejected: a typo would silently fall
  back to a default and change results).
- Consequences: renaming or removing a key is a breaking change of the
  configuration format.

## D-005 Canonicalization and hashing

- Status: Accepted
- Date: 2026-11-03
- Context: Reports, snapshots and ledgers are verified by recomputing hashes,
  also from other languages, so the bytes that get hashed must be defined
  independently of any one JSON library.
- Decision:
  - Canonical bytes are RFC 8785 (JSON Canonicalization Scheme) with every
    string and object key normalized to Unicode NFC first. Keys sort by UTF-16
    code units; numbers use the ECMAScript shortest form.
  - Input is rejected when keys repeat (also after NFC), when an integer lies
    outside ±(2^53 − 1), or when it is not JSON (NaN, infinities, unpaired
    surrogates). Harness scores that are NaN or infinite become missing
    (`invalid_output`) at import.
  - Snapshot files hold canonical records in primary-key order: by kind (run,
    item, label, judgment, preference, content), then by the kind's key
    fields, strings by code point. A file hash is over the lines joined with
    LF, without a trailing newline.
  - evalsig writes `blake3:<hex>` references and reads `blake3:` and
    `sha256:`. Numbers written to output envelopes are rounded to 10 decimal
    places before hashing.
  - Implementation: evalsig's own encoder in `evalsig-store` (no JCS crate),
    with `unicode-normalization`, `blake3` and `sha2`. The Python reference
    in `scripts/evalsig_canon.py` and the shared vectors in
    `tests/fixtures/canonicalization-vectors.json` must agree.
- Alternatives: an existing JCS crate (rejected: none adds NFC or the
  integer-range rule, and the encoder is small enough to own and test against
  the shared vectors); sha256 only (rejected: BLAKE3 is much faster on large
  snapshots, and sha256 remains accepted for verification).
- Consequences: changing any rule changes every hash, so it needs a new
  decision and a migration.

## D-010 Archive dependencies

- Status: Accepted
- Date: 2026-11-03
- Context: evalsig compresses its objects and reads Inspect `.eval` logs, which
  are zip archives whose entries are Zstandard (method 93) or Deflate. C code
  in the binary would complicate cross-compilation and the PyPI wheel.
- Decision:
  - Objects are zstd-compressed with `ruzstd` at `Fastest` and named by the
    BLAKE3 hash of the uncompressed bytes, under
    `objects/blake3/<2 hex>/<62 hex>.zst`. Writes go to a temporary file and
    are renamed into place; an existing object is never rewritten.
  - Inspect archives are read with `zip` with default features off and only
    `deflate`; Zstandard entries are taken raw (`by_index_raw`) and
    decompressed with `ruzstd`.
  - Measured on 2026-11-03 with 200,000 synthetic judgment lines (43.2 MB,
    each with a random 64-hex `output_ref`): `ruzstd` `Fastest` gives
    9.03 MB (ratio 4.78) at about 240 MB/s compressing and 710 MB/s
    decompressing; the `zstd` CLI gives 7.06 MB at level 1 and 7.04 MB at
    level 3. The 28 % larger objects are acceptable for local storage.
- Alternatives: the `zstd` crate (rejected for now: it builds the C libzstd;
  the stored format stays zstd, so switching later keeps old objects
  readable); zip's `zstd` feature (rejected: same C dependency).
- Consequences: object sizes are about a quarter larger than zstd level 3.
  Switch to the `zstd` crate if users report storage pressure.

## D-007 Ledger line format and the two ledgers

- Status: Accepted
- Date: 2026-11-03
- Context: The ledger is the evidence that conditions were fixed before
  results, how often a holdout was used and who changed labels. An agent can
  rewrite files in its own environment, so evidence must live where it cannot.
- Decision:
  - A ledger is append-only JSONL; each line follows
    `schema/ledger-entry.schema.json`: `seq` (from 0), `ts`, `actor`, `kind`,
    `ledger` (`agent` or `referee`), `inputs`, `outputs`, `summary`, `tool`,
    `prev` (the previous line's `hash`, `genesis` first) and `hash`.
  - `hash` is the BLAKE3 reference of the canonical bytes (D-005) of the line
    without `hash`. Verification also accepts `sha256:` lines.
  - Summaries hold no floating-point numbers; metric values are rounded
    strings, so no number formatting difference can change a hash.
  - Two chains: the agent-side ledger in `.evalsig/ledger.jsonl` and the
    referee ledger in the referee's environment. Each accepts only its kinds
    (audit spec §4); `split`, `exclude`, `label_revision` and `adjudication`
    occur in both (the referee's for holdout items).
  - Appends take an exclusive OS file lock (`std::fs::File::lock`), read the
    last line, write one line and sync, so concurrent writers serialize.
  - A chain detects edits to inner lines; truncation and a rebuilt chain are
    detected only against an anchor, the head hash recorded outside the
    ledger (`ledger anchor`).
- Alternatives: one ledger with an `actor` field (rejected: the agent could
  rewrite the referee's lines); a database table (rejected: its bytes are not
  stable to hash, audit spec §1).
- Consequences: the agent-side ledger is evidence, not proof; only the
  referee ledger and its results back a gate.

## D-008 CLI output envelope, verdict codes and exit codes

- Status: Accepted
- Date: 2026-11-03
- Context: CI jobs and AI agents act on evalsig's output, so its shape, its
  conclusions and its exit codes are a public API that must not shift under
  them.
- Decision:
  - Every command supports `--format json|md|junit|text` (default `text`).
    JSON output is one envelope with the keys `schema` (such as
    `evalsig.report/1`), `ok` (the command ran to the end; not a pass),
    `verdict` (`code` and `text`, or null), `results`, `warnings` (`code`,
    `message`), `next` (`command`, `why`), `provenance` (`snapshots`,
    `config`, `seed`, `tool`, `ledger`, `ledger_seq`) and `truncated`.
  - `verdict.code` is one of `pass`, `fail`, `improved`, `negligible`,
    `worsened`, `indistinguishable`, `drifted`, `stable`, `human_parity`,
    `below_human`, `insufficient_data`, `blocked`, `needs_decisions` and
    `budget_exhausted`. Verdicts follow only formulas and configuration.
  - Exit codes: 0 success or pass, 1 fail, 2 holdout budget exhausted,
    3 conditions invalid or not matching the approval or the candidate failed
    the referee's checks, 4 too few items to decide, 5 stopped by a validity
    check, 64 usage error.
  - Errors are `{"error": {"code", "message", "hint"}}` on standard error
    when JSON is requested (usage errors included), and
    `error[<code>]: <message>` plus `hint:` otherwise. Error codes are stable
    snake_case strings.
  - Compatibility: within a schema version, adding fields is compatible;
    changing a field's meaning, removing a field or adding a verdict code
    raises the version. Consumers treat unknown verdict codes as
    uninterpreted and quote them.
- Alternatives: per-command JSON shapes (rejected: agents would need a parser
  per command); clap's default exit code 2 for usage errors (rejected: 2 means
  an exhausted budget here, and 64 is the BSD `EX_USAGE` convention).
- Consequences: any change to these sets is a versioned, documented change.
