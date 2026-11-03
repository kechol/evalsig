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
    keys and fields of `crates/evalsig-cli/schema/records.schema.json`
    (printed by `evalsig schema records`). Unknown records and unknown
    fields are rejected.
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
    `crates/evalsig-cli/schema/ledger-entry.schema.json`: `seq` (from 0),
    `ts`, `actor`, `kind`, `ledger` (`agent` or `referee`), `inputs`,
    `outputs`, `summary`, `tool`, `prev` (the previous line's `hash`,
    `genesis` first) and `hash`.
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

## D-012 JSON Schema validation

- Status: Accepted
- Date: 2026-11-03
- Context: Users who write `records/1` directly need a schema to target and a
  validator whose error codes stay stable, so scripts and agents can act on
  them.
- Decision:
  - The schemas live in `crates/evalsig-cli/schema/` and are embedded in the
    binary; `evalsig schema <name>` prints them (`--raw` for the document
    alone).
  - `evalsig validate --records <file>` uses the `jsonschema` crate with
    default features off (no network retrieval, no TLS) and format checking
    on. Each line is validated against its record kind's definition, so
    errors name the failing keyword.
  - Issue codes: `json.invalid`, `json.duplicate_key`, `json.integer_range`,
    `record.not_object`, `record.missing_kind`, `record.unknown_kind`,
    `schema.<keyword>` (snake_case JSON Schema keyword, such as
    `schema.required`), and `record.<rule>` for the rules the schema cannot
    express (such as `record.level_out_of_range`). Invalid records give
    verdict `blocked` and exit code 5.
  - The licence allow list gains `MIT-0` (a dependency of `jsonschema`; more
    permissive than MIT).
- Alternatives: validating only through serde (rejected: errors would be
  serde messages without stable codes, and the published schema would not be
  what is enforced).
- Consequences: about 70 more crates in the CLI's dependency tree; the core
  library stays free of them.

## D-009 Numerical dependencies

- Status: Accepted
- Date: 2026-11-03
- Context: Every interval and test needs normal and Beta distribution
  functions accurate to the 1e-9 reference tolerance, and every resampling
  method needs random numbers that repeat from a recorded seed on every
  platform.
- Decision:
  - `statrs` (default features off, `std` only) provides the normal and Beta
    distribution functions and their inverses. Its Beta inverse matches the
    exact and Jeffreys interval reference values within 1e-9 (worst error
    about 4e-15 on 2026-11-03), so no fallback inverse is kept; the
    reference test fails if a later version drifts.
  - `rand_chacha` (default features off) provides ChaCha20, seeded with
    `seed_from_u64` from the configured or recorded seed. Uniform integers
    use Lemire's unbiased method and uniform doubles take 53 bits, both
    written in evalsig, so no distribution code of `rand` is involved.
  - Sums run in a fixed iteration order with Neumaier compensation, so
    results do not depend on threads or map order.
  - A test pins the first ChaCha20 outputs and a derived mean for a fixed
    seed; it passes on macOS and on Linux.
- Alternatives: `rand` with its distributions (rejected: its sampling
  algorithms may change between versions and alter results for a recorded
  seed); writing the special functions ourselves (rejected: statrs meets the
  tolerance and is widely used).
- Consequences: upgrading `statrs` or `rand_chacha` requires the reference
  and pinned tests to pass unchanged.

## D-023 Per-item rows as JSON Lines

- Status: Accepted
- Date: 2026-11-03
- Context: Python and notebook users take per-item comparison results into
  their own tools (SPEC §12.2: `compare --items --format jsonl`). The envelope
  formats of D-008 put all rows inside one JSON document.
- Decision:
  - `--format jsonl` is added to the formats of D-008. It prints the per-item
    rows of a command's `results.items`, one JSON object per line, and nothing
    else. In v1 only `compare --items` produces such rows; other commands
    reject `--format jsonl` as a usage error.
  - The four envelope formats of D-008 are unchanged; this is an addition
    under D-008's compatibility rule.
- Alternatives: a separate `export` command for item rows (kept for Parquet and
  CSV in a later version; a format flag is what scripts expect today).
- Consequences: commands that later gain `--items` may support `jsonl` too.

## D-006 Working store: SQLite

- Status: Accepted
- Date: 2026-11-03
- Context: The dashboard and interactive queries need indexed tables, while the
  canonical data are JSONL snapshots whose bytes are hashed. The working store
  must never become the evidence, and readers must not block on the CLI.
- Decision:
  - The working copy is SQLite through `rusqlite` with the `bundled` feature,
    at `.evalsig/cache/evalsig.db`. It holds STRICT tables per record kind
    (key columns plus the record as JSON) and the snapshot list.
  - Triggers refuse UPDATE and DELETE on every table; the copy changes only by
    `evalsig rebuild`, which deletes it and rebuilds it from the canonical
    snapshots named by the ledger.
  - WAL journal mode, so the dashboard can read while the CLI writes.
  - Nothing hashed or verified reads the working copy; reports and `verify`
    use the snapshots.
- Alternatives: DuckDB (rejected for v1: another process cannot open a file
  being written even read-only, and SQLite aggregates 600,000 rows in
  0.25–0.55 s, enough for interactive use; Parquet export serves larger
  analyses); no working copy (rejected: the dashboard would rescan snapshots on
  every request).
- Consequences: the bundled SQLite is C code compiled into the binary, the one
  exception to keeping C out (D-010); `cargo-deny` accepts its licences.

## D-024 Content storage modes and purge

- Status: Accepted
- Date: 2026-11-03
- Context: Inputs, outputs and grader rationales can hold personal data or
  secrets, and records name them by hash so the text can go without breaking
  snapshots or ledgers (SPEC §9 "Content"). The free-text fields of records
  (`item.attrs`, `label.reason`, `missing.detail`, strata values) live inside
  immutable snapshots, so removing text from them needs a new snapshot.
- Decision:
  - A record's `*_ref` is always the hash of the original body, whatever
    `store.content` is, so how bodies are kept never changes what records
    name. `full` stores the body under that name; `hash` stores nothing;
    `redact` stores the body with every match of `store.redact_patterns`
    replaced by `[redacted]`, under the redacted bytes' own name, and the
    snapshot's provenance keeps the pair (original, stored). Under `redact`,
    imports also replace matches in `item.attrs` strings, strata values,
    `label.reason` and `missing.detail` before the snapshot is written, so
    those snapshots hold the redacted fields.
  - `evalsig purge <ref> --reason <text> --yes` finds the items whose records
    name the body, replaces those items' free-text fields in every current
    snapshot with the BLAKE3 reference of the value, writes the new
    snapshots, and appends a `purge` line whose summary lists each
    replacement (`replaced: [{from, to, provenance}]`). It then deletes the body, its
    redacted copy, the superseded snapshots and the saved reports that name
    them, and rebuilds the working copy if one exists. Superseded snapshots
    remain only as hashes in the ledger. Purge is the one operation that
    removes objects (D-010).
  - Data loading follows `purge` lines: a replacement takes the place of the
    snapshot it supersedes, also when that snapshot is imported again, and
    importing records that were purged is refused.
  - `regex` (MIT OR Apache-2.0, already in the tree through `jsonschema`) is a
    direct dependency of `evalsig-store` for the patterns.
- Alternatives: naming records after the redacted body (rejected: the
  references and input-change detection would depend on the storage mode);
  a new `content` field for the stored name (rejected: `records/1` is a
  public format, D-003); purging only the body (rejected: the same text often
  appears in attrs, reasons and details).
- Consequences: hashes of short or guessable values (a stratum such as
  `chat`, a name) can be recovered by guessing; purge removes the text, not
  the possibility of confirming a guess. Reports computed before a purge can
  no longer be verified. Strata values replaced by hashes put the purged items
  in strata of their own.

## D-011 Declarative mapping file `mapping.toml` (`mapping/1`)

- Status: Accepted
- Date: 2026-11-03
- Context: Records from custom harnesses rarely match `records/1`. A
  declaration that a person can read, that is hashed into provenance and that
  an approval can pin is preferable to a converter script (SPEC §5.2 ②, §5.3).
  Users write it by hand, so its shape is a public contract.
- Decision:
  - `mapping.toml` is TOML with `schema = "evalsig.mapping/1"`, `name`,
    `[source]` (`format` json, jsonl or csv; `records_path`; `expect`;
    `fingerprint`), `[run]`, `[item]`, `[judgment]` (with `explode`,
    `features`, `missing_if`), an optional `[label]` (with `join`) and
    `[[decisions]]`, with the fields of
    `crates/evalsig-cli/schema/mapping.schema.json` (`evalsig schema mapping`,
    `$id` `urn:evalsig:schema:mapping:1`). Unknown fields are rejected.
  - Paths are a JSONPath subset: `$` is the source row (in `join.path`, the
    joined file), `@` the current element; `.name`, `["any key"]`, `[n]`,
    `[*]` and `[?(@.x)]` / `[?(@.x == literal)]` segments; `{name}` inserts an
    `explode` variable and `{$.x}` / `{@.x}` the text of a value, also inside
    names and templates.
  - An expression is a path, `DECIDE`, or a table with one source (`path`,
    `template`, `arg`, `const`, or none for the current element) followed by
    `value_map`, `default`, `offset` and `type`. A judgment or label value that
    cannot be mapped becomes `missing` with reason `invalid_output`, never 0; a
    key or required field that cannot be mapped stops the import.
  - Fields that name bodies (`input_ref`, `reference_ref`, `output_ref`,
    `rationale_ref`) take the body; the import stores it under
    `store.content` (D-024) and writes its hash and a `content` record.
  - `evalsig import --mapping <file> [--arg name=value]... <files>` applies a
    mapping; `evalsig import --from <name>` applies an `[[import]]` block of
    the configuration, read as the equivalent mapping (paths for JSON, column
    names for CSV), with one run record per run ID and the block name as the
    system ID. The mapping file's hash is the provenance `mapping`; joined
    files are provenance sources. Any `DECIDE` left stops the import with
    exit code 5 and lists where it is.
- Alternatives: a full JSONPath or jq dependency (rejected: a larger surface
  than the six operations SPEC §5.2 needs, and a third-party grammar would
  become part of the public format); converter scripts only (rejected: not
  readable or pinnable, SPEC §5.2).
- Consequences: changing a field or the path grammar is `mapping/2`. Shapes the
  subset cannot express still need a converter (`import --converter`).

## D-017 Alternative annotator test parameters

- Status: Proposed
- Date: 2026-11-03
- Context: `agree --parity` answers whether a system could replace one of the
  human raters (metrics spec §9.4). The spec gives ε = 0.1, a winning rate of
  0.5 to pass and the null "the system is worse by ε or more", but asks for
  the agreement measure, the default ε and the minimum counts to be checked
  against the source, Calderon, Reichart and Dror, "The Alternative Annotator
  Test for LLM-as-a-Judge" (ACL 2025, arXiv:2501.10970), before
  implementation. The authors' reference procedure (the `alt_test` function
  in the example notebook of github.com/nitaytech/alttest, read on
  2026-11-03) settles these; the repository has no licence, so evalsig takes
  the procedure, not the code or the data.
- Decision (proposed):
  - For each human rater j with enough items, and each item i that j and at
    least one other human labelled: the score of a labeller on i is its
    agreement with the other humans on i, the share of them it matches for
    binary and nominal labels, minus the root-mean-square difference for
    ordinal and score labels. W_sys,i = 1 when the system's score is at
    least j's, W_j,i = 1 when j's score is at least the system's; ties count
    for both.
  - Per rater, a one-sided one-sample t-test of d_i = W_j,i − W_sys,i with
    H0: mean(d) ≥ ε against mean(d) < ε. Rejecting H0 means the system wins
    against j. The burden of proof stays with the system.
  - Benjamini-Yekutieli over the raters at q = 0.05. The winning rate ω is
    the share of tested raters the system wins against; `human_parity` when
    ω ≥ 0.5, `below_human` otherwise. The advantage probability ρ, the mean
    over raters of mean(W_sys), is reported beside ω for choosing between
    systems.
  - ε defaults to 0.1, the paper's rule of thumb for crowd raters and the
    strictest of its three (experts 0.2, skilled raters 0.15); the setting
    `agree.parity_epsilon` accepts other values and the output states the ε
    used.
  - A rater with fewer than 30 items shared with other humans is not tested
    and is listed as skipped; with fewer than two tested raters the verdict
    is `insufficient_data`. Three or more human raters are recommended;
    with two the same procedure runs (each is compared with the other) and
    the output warns that the result is less reliable. This replaces the
    spec's separate two-rater bootstrap variant.
  - The reference case comes from an independent implementation in
    `scripts/reference_values.py` on synthetic labels, since the authors'
    data carry no licence.
- Alternatives: the code's default ε = 0.2 (not taken: it is the expert-rater
  value and makes parity easier to claim for the crowd-labelled sets most
  users have); a Wilcoxon signed-rank test below 30 items, which the paper
  mentions (not taken: the reference procedure skips such raters instead,
  and one rule is easier to state); the spec's two-rater paired bootstrap
  (not taken: the paper's procedure already covers two raters and one
  method keeps the human–human and system–human comparisons alike).
- Consequences: a parity claim needs about 30 shared items per rater, so
  small label sets return `insufficient_data` rather than a weak verdict.
  Changing ε changes the verdict, so it is part of what a report records.
  Agreement for ordinal labels uses the RMSE score, not weighted κ.
