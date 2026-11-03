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

## D-016 Whether the `latent` reference ships in v1

- Status: Proposed
- Date: 2026-11-03
- Context: `latent` replaces the raters' positive share with a Dawid-Skene
  posterior, estimating each rater's sensitivity and specificity by EM
  (metrics spec §9.2). The spec recommends shipping it with three or more
  raters per item and asks for a simulation of bias and interval coverage
  against `soft` first (SPEC §15). `scripts/sim_latent_reference.py`
  (seeded; 200 datasets × 200 bootstrap resamples, the EM re-estimated in
  each) labels 300 items with a crossed panel of rater accuracy 0.85, varies
  raters per item (3, 5), rater bias (sensitivity and specificity ± 0, ± 0.1)
  and the share of hard items on which every rater says positive with
  probability ½ (0, 0.2), and measures the system's H and F (true 0.8 and
  0.1) against the true class. Result (mean bias; coverage of the 95%
  bootstrap interval):

  | raters | bias | hard | H bias soft / latent | H cover soft / latent | F cover soft / latent |
  |---:|---:|---:|---|---|---|
  | 3 | 0 | 0 | −0.20 / −0.11 | 0.00 / 0.42 | 0.28 / 0.46 |
  | 3 | 0.1 | 0 | −0.23 / −0.10 | 0.00 / 0.43 | 0.54 / 0.54 |
  | 3 | 0 | 0.2 | −0.28 / −0.22 | 0.00 / 0.01 | 0.01 / 0.09 |
  | 5 | 0 | 0 | −0.21 / −0.05 | 0.00 / 0.92 | 0.16 / 0.97 |
  | 5 | 0.1 | 0 | −0.22 / −0.04 | 0.00 / 0.97 | 0.35 / 0.99 |
  | 5 | 0.1 | 0.2 | −0.29 / −0.17 | 0.00 / 0.07 | 0.00 / 0.34 |

  `soft` estimates performance against one rater drawn at random, a
  different estimand, so its gap to the true class is expected; the question
  is whether `latent` delivers the true-class value it claims.
- Decision (proposed): `latent` does not ship in v1. `--reference latent`
  and `reference.mode = "latent"` stay recognised and are refused with the
  existing "not available in this version" error, and `soft` remains the
  default with its estimand stated in the output. With three raters per
  item, the minimum the spec names, the latent intervals cover the
  true-class H in fewer than half of the datasets, and with a fifth of the
  items hard for everyone in under a tenth; only five raters and no
  difficulty-driven splits give near-nominal coverage. Shipping it would
  print confident true-class numbers that are wrong more often than not for
  the label sets most users have.
- Alternatives: ship with the spec's three-rater minimum and the
  overconfidence note (not taken: the note cannot fix intervals that miss
  more than half the time); ship with a five-rater minimum and a warning
  when split items are common (not taken for v1: hard items cannot be told
  apart from rater noise in the data, so the warning would not catch the
  failing case; a candidate for v2 together with per-rater error rates in
  `agree`).
- Consequences: the `latent_reference_unidentified` check and the EM code
  are not built in v1. Users who need true-class estimates rather than
  agreement with a random rater have no built-in method; the report's
  estimand line says what `soft` measures. Per-rater sensitivity and
  specificity, which `latent` would also have given, are not reported.

## D-022 Two-level split

- Status: Accepted
- Date: 2026-11-03
- Context: Fixing a system needs items to look at (dev), items to decide
  adoption on (validation) and items the agent never sees labelled
  (holdout). The holdout must be cut and sealed with the judge's authority,
  while dev and validation splits are the agent's own business: their labels
  are already in its environment (SPEC §8.0, §8.1). The agent must still
  avoid splitting into items the judge has sealed, without learning their
  labels.
- Decision:
  - Splitting has two levels. `split --holdout` and `seal` belong to the
    judge (M5). `split` (dev / validation) runs with the agent's authority and
    records a `split` line in the agent-side ledger.
  - The unit of every split is the cluster (`item.cluster`, else the item).
    Methods: `time` (cut at an `occurred_at` boundary, by each cluster's
    earliest item), `random` (stratified by a named item stratum when given,
    seeded), and `assign` (a user file of `item_id` → `dev`/`validation`).
    The default is `time` when every item has `occurred_at`, else `random`.
    A user assignment that splits one cluster is refused
    (`split_crosses_cluster`).
  - The judge hands the agent the items left after the holdout as a file,
    `remaining.json`: `{"schema": "evalsig.remaining/1", "holdout": <set
    hash>, "items": [<item IDs>]}`. It names IDs only, never labels.
    `split --within remaining.json` splits only those items, records the
    holdout hash in the `split` line, and refuses an `assign` file or an
    explicit item that is not in the list. Without the file, `split` uses all
    items and states that no holdout is excluded.
  - A split is identified by the BLAKE3 reference of the canonical JSON
    array of its sorted item IDs, the same hash `compare` uses for
    `validation_uses`. The ledger line records each part's hash and size, the
    method, the seed and the holdout hash. `split` writes the assignment to
    `splits/<hash>.json` in the project; `splits` lists the recorded splits.
  - `compare --split validation` restricts a comparison to the latest
    split's validation items.
- Alternatives: fetching the remaining set from `gate serve` (not taken:
  the CI deployment has no running server, and a file works for both and is
  auditable); giving the agent the holdout IDs (not taken: the agent only
  needs what it may use, and a list of what is withheld invites fitting to
  it); splitting by item (not taken: items of one document in two parts leak
  across the split).
- Consequences: the agent must keep `remaining.json` current after the
  judge re-cuts the holdout; a stale file is detected only by the judge when
  a gate submission covers sealed items. Splits are per project, not per
  task.

## D-021 `check.toml` as a subset of `gate.toml`

- Status: Accepted
- Date: 2026-11-03
- Context: Teams want an interval-based pass/fail in CI before they set up
  a judge, a sealed holdout and preregistration (SPEC §3.2, §10.1). A second
  condition language would drift from `gate.toml`; one that accepted gate-only
  sections would let a check pass for a gate.
- Decision:
  - `check.toml` uses the `gate.toml` keys `name`, `estimand`, `min_items`,
    `replicates`, `positive`, `decision`, `[reference]`, `[grader]` and
    `[[conditions]]`, with the same meaning. `hypothesis`, `split`,
    `[compare]`, `[holdout]` and `budget` are refused, as are unknown keys.
    Settings it leaves out come from `config.toml`.
  - A check judges one run against absolute criteria. Its rules are the two
    interval rules on a single run, `lower_bound_at_least` and
    `upper_bound_at_most`; the difference rules, `not_worse_than` and `pass`
    need arms or a referee and stay with `gate`. Every condition states
    `sided` (`one` or `two`); a one-sided bound at level L is the matching end
    of the two-sided interval at 2L − 1. `missing_rate` counts every
    system-side missing judgment and uses the exact interval.
  - All conditions must hold (intersection-union). Exit codes: 0 all hold,
    1 one fails, 4 a condition has no interval or the run has fewer than
    `min_items` items, 5 a validity check stops. The output carries
    `preregistered: false`, and the verdict text says it is not
    preregistered.
  - `--format junit` writes one test case per condition; `gate` reuses it.
- Alternatives: a separate check syntax (not taken: two languages for the
  same rules drift apart); allowing delta rules against a named baseline
  (not taken: that is a comparison, which `compare` and `gate` already
  cover with their own guards).
- Consequences: moving a check to a gate means adding the gate-only sections
  and preregistering, not rewriting the conditions.

## D-025 Export formats and the Parquet dependency

- Status: Accepted
- Date: 2026-11-03
- Context: `evalsig export` hands the data to tools built for large analysis,
  such as DuckDB, as Parquet, CSV or canonical JSONL (SPEC §9 "Large
  analysis", §10.1). The exports are derived: verification always runs on the
  canonical snapshots. Writing Parquet by hand would mean re-implementing its
  Thrift metadata and page encoding.
- Decision:
  - `export --to parquet|csv|jsonl --out <dir>` writes one table per record
    type (`runs`, `items`, `judgments`, `labels`, `preferences`), skipping
    empty ones. Columns are the record fields, nested values (strata, cost,
    features, system, grader) as canonical JSON text, and every column is
    nullable. `jsonl` writes the records in canonical form instead, one file
    per type. `--items <run>` adds that run's per-item report rows as `items_<run>`.
  - `parquet` 60 (Apache-2.0, the Apache Arrow Rust implementation) with
    default features off is a dependency of `evalsig-cli` only. Files use its
    low-level column writer, are uncompressed and carry no Arrow schema; this
    adds `bytes`, `chrono`, `half`, `num-bigint`, `seq-macro` and `zerocopy`
    (MIT or Apache-2.0) and no C code. CSV is written by hand (RFC 4180
    quoting).
- Alternatives: the `arrow` feature and compression codecs (not taken: they
  pull in a large tree and C code for zstd and lz4, for files that are derived
  and short-lived); the `duckdb` crate (not taken: a bundled C++ engine for a
  writer); Parquet by hand (not taken: error-prone for a format other tools
  must read).
- Consequences: Parquet files are larger than compressed ones. Readers that
  want an Arrow schema infer it from the Parquet types.

## D-026 Licence exception for `tiny-keccak`

- Status: Accepted
- Date: 2026-11-03
- Context: `parquet` (D-025) depends on `ahash` with `compile-time-rng` when the
  target is `wasm32`, which brings `const-random`, its proc-macro and
  `tiny-keccak` (CC0-1.0). cargo-deny checks every target, so the licence check
  fails. D-025 listed only the crates of the native targets; the lock file also
  gains `const-random`, `const-random-macro`, `tiny-keccak`, `crunchy`,
  `getrandom` and target-specific `windows-*` crates, all MIT or Apache-2.0
  apart from `tiny-keccak`.
- Decision: `deny.toml` allows CC0-1.0 for the crate `tiny-keccak` only, as a
  licence exception, not in the general allow list. It runs at compile time
  inside a proc-macro on `wasm32` and is never part of an evalsig binary;
  CC0-1.0 places no condition on redistribution in any case.
- Alternatives: restricting cargo-deny's graph to the shipped targets (not
  taken: D-020 has not fixed the targets, and a narrower graph would hide other
  wasm-only crates without saying so); dropping Parquet (not taken: the spec
  names it for large analysis).
- Consequences: a new CC0 crate fails the check again and needs its own entry.

## D-014 The referee's home, authority and `gate serve` transport

- Status: Accepted
- Date: 2026-11-03
- Context: Splitting off the holdout, sealing, approving conditions, excluding
  holdout items and gating need the referee's authority, which the agent must
  not have (SPEC §8.0, §8.5). The spec places the referee under another OS user,
  in CI or on another machine, reached by a Unix socket or token HTTP on
  127.0.0.1, and asks for a warning when it runs with the agent's privileges.
  It leaves open where the referee keeps its state and how a command knows it
  has that authority.
- Decision:
  - The referee works from a home directory named by `--referee <dir>` or
    `EVALSIG_REFEREE`. `evalsig referee init <dir>` creates it with an age
    X25519 identity (`identity.txt`, mode 0600) and its recipient
    (`recipient.txt`), the referee ledger (`ledger.jsonl`, D-007 side
    `referee`), and `holdouts/`, `sealed/`, `approved/`, `results/` and
    `state/`. The home is a plain directory, so CI can restore it from a
    protected store.
  - Authority is possession: a referee-only command needs a referee home whose
    identity the process can read, and fails with exit 3 and code
    `referee_only` otherwise. The agent's project never holds a referee home.
  - The seal is void, and `gate` says so in its output and its ledger line,
    when the referee home lies inside the project directory or its identity is
    readable by group or others.
  - `gate serve` listens on a Unix socket `gate.sock` in the referee home
    (mode 0600; the referee grants access by group) and, when asked, on token
    HTTP bound to 127.0.0.1. The token is 32 random bytes, hex, in `token`
    (mode 0600), compared in constant time from an `Authorization: Bearer`
    header. A submission carries the candidate's records and the approved
    hash; the response carries only what SPEC §8.3 and §11.3 allow, and the
    full result goes to `results/` and the referee ledger.
- Alternatives: the OS keychain (not taken: unavailable in CI and on Linux
  servers without a session); mutual TLS (not taken: certificate handling for a
  local, single-host channel; HTTPS for a remote referee is left to a reverse
  proxy); checking the OS user (not taken: CI runs everything as one user; the
  key is what separates authority).
- Consequences: anyone who can read the identity file is the referee; the
  deployment guide (docs/referee.md) says how to keep it from the agent.

## D-013 Sealing with age (X25519)

- Status: Accepted
- Date: 2026-11-03
- Context: Holdout labels must be encrypted so only the referee can read them,
  in a format people can check without evalsig (SPEC §8.2, audit §7). The
  crate survey (`research/rust-crates.md` §4) chose the `age` crate for its
  specified format, which the Go reference implementation and the `rage` CLI
  read, over `hpke` or `crypto_box`, whose file format evalsig would have to
  invent.
- Decision:
  - Sealed files are age v1 files to the referee's X25519 recipient, written
    with the `age` crate 0.11 (MIT OR Apache-2.0) with default features off, a
    dependency of `evalsig-store` only, used by its sealing module and by
    `referee init` for the identity. They decrypt with `age -d -i` or
    `rage -d -i`.
  - 0.11 rather than the survey's 0.12: 0.12 adds P-256 and post-quantum
    recipients through `p256`, whose `crypto-bigint` 0.5 carries
    RUSTSEC-2026-0387 (non-constant-time operations). X25519 is all evalsig
    uses, and 0.11 has no such dependency.
  - age's localisation proc-macro (`i18n-embed-fl`) brings `basic-toml`
    (RUSTSEC-2026-0374) and `proc-macro-error2` (RUSTSEC-2026-0173), both
    flagged unmaintained, at compile time only. `deny.toml` ignores those two
    advisories with this reason; vulnerabilities stay denied.
- Alternatives: age 0.12 (not taken: the advisory above); `hpke` or
  `crypto_box` with an evalsig format (not taken: no outside tool could
  check the files); shelling out to the `age` CLI (not taken: one more
  program to install and trust on every referee).
- Consequences: moving to age 0.12 or later means re-checking advisories; the
  file format does not change between them.

## D-015 Ladder defaults: step factor k and budget

- Status: Proposed
- Date: 2026-11-03
- Context: `gate` returns a value only when the candidate sets a new best,
  rounded to a step η = max(k·√(ψ̂/n), 1/n) toward the baseline (the Ladder,
  metrics spec §11.3). The spec proposes k = 2 and a budget B = 5 but asks
  for a simulation first: the false pass rate when an agent picks its next
  candidate from the gate's answers, and the share of real improvements
  that pass and get their value back (SPEC §15). `scripts/sim_ladder.py`
  runs that simulation: n of 300 and 1,000 in the primary denominator,
  B of 5, 10 and 20, k of 1, 2 and 3, α = 0.05, validation error 0.1
  (ψ̂ = 0.2), 4,000 runs per cell, seed 20261103. The attack is Blum and
  Hardt's: B − 1 random candidates with no real advantage, then the
  item-by-item majority of the ones the answers favour.
- Results (`uv run --project scripts scripts/sim_ladder.py`):

  | n | B | false pass, full feedback | false pass, Ladder k = 2 | real 0.05 passes | value back, k = 2 / k = 3 |
  |---:|---:|---:|---:|---:|---:|
  | 300 | 5 | 0.067 | 0.042 | 0.347 | 0.346 / 0.128 |
  | 300 | 10 | 0.121 | 0.048 | 0.270 | 0.270 / 0.138 |
  | 300 | 20 | 0.239 | 0.045 | 0.203 | 0.203 / 0.135 |
  | 1000 | 5 | 0.063 | 0.036 | 0.893 | 0.893 / 0.706 |
  | 1000 | 10 | 0.108 | 0.044 | 0.850 | 0.850 / 0.697 |
  | 1000 | 20 | 0.233 | 0.047 | 0.783 | 0.783 / 0.703 |

  Under the Ladder the false pass rate stays at or below α for every k and
  B tried; with the unrounded difference returned it exceeds α already at
  B = 5 and grows with B. The step factor barely moves the false pass rate,
  because the Ladder's significance requirement withholds almost every
  value from a candidate with no real advantage; it moves whether a real
  improvement learns its value. k = 1 and k = 2 return the value of every
  real improvement that passes; k = 3 withholds it from about two thirds at
  n = 300.
- Decision (proposed):
  - `reuse_guard = "ladder"` is the default and `ladder_k` defaults to 2
    (at least 1). At approval the referee fixes η from k, the primary
    condition's denominator n and ψ̂ = max(0.05, 4/n, min(1, 2·e_val)),
    with e_val the baseline's validation error in that denominator, and
    records η in the `preregister_approve` line. Example: e_val = 0.1,
    n = 300 gives η ≈ 0.052.
  - A user `ladder_step` is accepted only when it is at least the automatic
    η; a smaller one fails approval as a usage error.
  - At approval the referee evaluates the baseline on the holdout as the
    first best; this does not use the budget.
  - A submission returns a value only when the lower bound of its
    difference to the best, one-sided at α/B, is above 0 and the
    difference is at least η. The value is rounded to a multiple of η
    toward the baseline (a delta rule rounds Δ). Otherwise the output
    carries no value, only the verdict, the per-condition verdicts and the
    budget left.
  - `holdout.budget` defaults to 5. Each submission is judged at α/B, so a
    larger budget costs power: at n = 300 a real 0.05 improvement passes
    35% of the time at B = 5 and 20% at B = 20.
- Alternatives: k = 1 (not taken: no measurable gain over k = 2 in these
  runs, and twice as many returned values leak more against stronger
  attacks than this one); k = 3 (not taken: withholds the value from most
  real improvements at a few hundred items); B = 10 or 20 by default (not
  taken: the power loss is larger than the protection gained, since the
  Ladder already keeps the false pass rate near α); returning the
  unrounded difference (not taken: the false pass rate exceeds α from
  B = 5).
- Consequences: at a few hundred holdout items the Ladder's theoretical
  guarantee does not hold (it needs steps near (ln B / n)^{1/3}); the
  protection rests on the budget, α/B and the withheld values, and
  `docs/referee.md` says so. The simulation covers one attack; a stronger
  one could do better against k = 1 or 2.

## D-027 Item exclusion (`exclude`)

- Status: Accepted
- Date: 2026-11-03
- Context: Some items should not count at all (a duplicate, a broken input,
  an item outside the population, one that must not be evaluated). Leaving
  them out by hand breaks the record of what an analysis used, and leaving
  them out of the holdout must stay with the referee (SPEC §10.1, audit
  spec §4: the `exclude` line carries the reason code and the count; holdout
  items are the referee's).
- Decision:
  - `evalsig exclude <item>... --reason <code> [--detail <text>]`. The
    reason codes are `duplicate`, `invalid_input`, `out_of_scope`,
    `privacy` and `other`; `other` needs `--detail`.
  - On the agent's side the exclusion works like `purge` (D-024): every
    snapshot in force that holds records of the items is replaced by one
    without them, and an `exclude` line in the project ledger lists the
    replacements, the reason, the detail and the count; its input is the
    BLAKE3 reference of the sorted item IDs, whose JSON array is kept in
    the object store. Earlier reports keep naming the superseded snapshots,
    so `verify` still reproduces them; nothing is deleted. Records of the
    items imported later come back in, and the output of `exclude` says so.
  - When a split in the project was made within the referee's remaining
    items, an item outside them is a holdout item and the agent-side
    `exclude` refuses it (`holdout_item`, exit 3).
  - With `--referee <dir>` the referee excludes holdout items only: it
    writes `exclusions/<hash>.json` in its home and an `exclude` line in
    its ledger, and from then on the holdout's items, for `seal`,
    `preregister approve` and `gate`, leave those items out. An exclusion
    made after approval changes the pinned holdout items, so `gate`
    refuses that approval (exit 3) until the referee approves again; the
    budget stays with the seal.
- Alternatives: a filter applied when data is loaded (not taken: `verify`
  recomputes a report from its snapshots and would need the exclusions as
  of the report as well); free-text reasons (not taken: the ledger line
  must carry a code that can be counted); letting the agent's exclusions
  reach the referee (not taken: the referee trusts only what it fixed).
- Consequences: a later import of an excluded item's records brings them
  back, so an exclusion is repeated after such an import. The item list
  lives in the object store, not in the ledger summary.

## D-018 Agent mode and the deferral of MCP

- Status: Accepted
- Date: 2026-11-03
- Context: Coding agents call the CLI from a shell and read its output into
  a limited context. They need output that fits, that keeps the text of
  evaluated systems apart from evalsig's own words, and that never waits
  for an answer (SPEC §12.3, agent-interface spec §2.1, §2.4, §2.5, §4).
  The spec leaves open how the mode is triggered and how output is cut.
- Decision:
  - Agent mode is on with `--agent`, or when `AI_AGENT` is set and not
    empty, standard output is not a terminal and no `--format` is given
    (the condition DuckDB 2.0's CLI uses). `--no-agent` or
    `EVALSIG_AGENT_MODE=0` turns it off; the flags win over the variables.
    `--agent` with `--format` is a usage error.
  - Agent mode changes the output only. It prints one JSON envelope on
    standard output and errors as JSON on standard error; exit codes,
    verdicts, ledger lines and every referee check are the same as without
    it.
  - Each top-level array in `results` shows at most 50 rows from
    `--cursor <n>` (default 0); strings longer than 500 characters end in
    `…`; when the envelope still exceeds 20 KB, the row limit halves until
    it fits. `truncated` then gives `rows_omitted`, `cells_cut`,
    `next_cursor` when rows remain, and `hash`, the BLAKE3 reference of the
    canonical JSON of the full `results`. `--format json` is never cut.
  - Text that comes from evaluated data (item strata, and the values that
    `map probe`, `map draft` and `map test` quote) is left out in agent
    mode, with the warning `content_withheld`. With `--include-content` it is returned in
    `untrusted_content`, beside `content_notice`, and never in `results`.
  - evalsig asks no questions. In agent mode, an operation that changes
    the procedure (`preregister --draft`) also needs `--yes`, and the
    ledger's actor comes from `AI_AGENT` as `agent:<name>` unless
    `EVALSIG_ACTOR` is set.
  - No MCP server in v1. If one is built, its tools are generated from the
    same command definitions as `help --json`.
- Alternatives: switching on `AI_AGENT` alone (not taken: a person piping
  to a file with the variable set would lose rows); cutting `--format json`
  as well (not taken: scripts need the full result); an MCP server now (not
  taken: agents with a shell use the CLI, and a second tool surface would
  have to be kept in step).
- Consequences: an agent that needs every row pages with `--cursor` or asks
  for `--format json`. `--cursor` has no effect outside agent mode.
