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

- Status: Superseded by D-029
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

- Status: Superseded by D-036
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

- Status: Superseded by D-028
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

- Status: Accepted
- Date: 2026-11-06
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
- Decision:
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

- Status: Accepted
- Date: 2026-11-06
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
- Decision: `latent` does not ship in v1. `--reference latent`
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
  in a format people can check without evalsig (SPEC §8.2, audit §7). A
  survey of the encryption crates chose the `age` crate for its
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

- Status: Accepted
- Date: 2026-11-06
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
- Decision:
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
  - Text that comes from evaluated data (item strata, results keyed by
    stratum values, the values that `map probe`, `map draft` and `map test`
    quote, and the issue messages and paths of `import` and `validate`) is
    left out in agent mode, with the warning `content_withheld`.
    Identifiers (item, run, rater, system) stay, since every answer needs
    them. A table names these fields for every envelope
    schema; a schema missing from it has its whole `results` treated as
    content. With `--include-content` it is returned in
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

## D-019 Dashboard stack and access control

- Status: Accepted
- Date: 2026-11-04
- Context: `evalsig dashboard` serves read-only pages of the same results the
  CLI computes, to one person on their machine (SPEC §11). It must not be
  reachable by other sites through the browser (DNS rebinding) and must show
  exactly the CLI's numbers. The planned stack was axum, tokio, rust-embed and
  maud; `gate serve` has since been built on the standard library (D-014).
- Decision:
  - The server is the standard library's: one thread per connection, GET
    only, bound to 127.0.0.1. It accepts a request only when its Host header
    is `127.0.0.1:<port>` or `localhost:<port>` and its URL carries the token
    made at each launch (32 random bytes, hex, compared in constant time);
    other requests get 403 or 421 and no page. Responses carry
    `Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline';
    img-src data:` and `X-Content-Type-Options: nosniff`.
  - Pages and charts are rendered on the server by `evalsig-dashboard` from
    the CLI's result JSON: HTML through an escaping writer, charts as SVG.
    The browser computes nothing and loads nothing from outside; no
    JavaScript is needed for the v1 screens (folding uses `<details>`).
  - The dashboard reads the project's canonical snapshots and never writes;
    it does not open the SQLite working copy.
  - The static HTML report (`report --format html`) uses the same renderer
    and embeds its data as `<script type="application/json">`.
- Alternatives: axum with tokio (not taken: about a hundred more crates and
  an async runtime for a single-user, read-only, loopback server whose
  requests the standard library handles); maud (not taken: a macro DSL for
  a few templates that an escaping writer covers); rust-embed (not taken:
  `include_str!` embeds the few assets).
- Consequences: no new dependencies. The HTTP handling is minimal by design
  and must stay GET-only and loopback-only; anything more (uploads, label
  editing in v2) needs this entry revisited.

## D-020 Distribution channels and supply-chain guarantees

- Status: Accepted
- Date: 2026-11-04
- Context: evalsig claims to be a referee, so a user must be able to check
  where a binary came from (SPEC §14). The channels are cargo, Homebrew,
  GitHub Releases and a PyPI wheel; macOS and Linux first, Windows later
  (T-113).
- Decision:
  - cargo-dist (pinned in `dist-workspace.toml`) builds the release from a
    tag on GitHub Actions for aarch64 and x86_64 macOS and Linux, with a
    SHA-256 checksum per artifact, a shell installer and GitHub artifact
    attestations for every artifact.
  - The documented install is `cargo install --locked evalsig-cli`; the MSRV
    is the workspace `rust-version`.
  - Homebrew, the PyPI wheel (maturin `bin`), crates.io and Windows are
    T-111, T-112 and T-113.
- Alternatives: hand-written release workflows (not taken: cargo-dist
  already produces checksums, installers and attestations from one
  config); signing with a project key (not taken: attestations tie an
  artifact to the workflow run without a key to guard).
- Consequences: a release needs a pushed tag and GitHub's attestation
  service, so the first verifiable artifacts come from that external step.

## D-028 No SQLite working copy

- Status: Accepted
- Date: 2026-11-06
- Supersedes: D-006
- Context: D-006 kept a SQLite copy of the data in force so the dashboard
  could query indexed tables. D-019 then settled that the dashboard reads
  the canonical snapshots and never opens the copy. Since then nothing reads
  it: `import`, `purge` and `exclude` keep it current and `rebuild` prints
  its row counts. It costs the one piece of C code in the store (D-010) and
  a maintenance step in every command that replaces snapshots.
- Decision:
  - There is no working copy. `.evalsig/` holds `config.toml`, the ledger,
    `objects/`, `reports/` and the directories commands create on demand;
    `init` no longer creates `cache/`.
  - `evalsig rebuild` is removed. `purge` and `exclude` replace snapshots
    and write their ledger lines, with no further step.
  - The dashboard reads the data in force once and keeps it while the
    project ledger's head is unchanged, so a page does not reread the
    snapshots on every request.
  - `evalsig-store` no longer depends on `rusqlite`. `evalsig-adapters`
    keeps it, read-only, for promptfoo databases (SPEC §13.1).
- Alternatives: keeping the copy for later interactive queries (not taken:
  no consumer exists, and Parquet export serves analyses elsewhere); keeping
  `rebuild` as a no-op (not taken: v1 is unreleased, so nothing depends on
  the command).
- Consequences: the bundled SQLite stays in the binary only through the
  promptfoo adapter. Anything that needs indexed queries later decides its
  own store.

## D-029 Application and referee crates

- Status: Accepted
- Date: 2026-11-06
- Supersedes: D-001
- Context: D-001 placed everything between the stores and the command line
  in the binary crate. The dashboard therefore reaches the analyses by
  rebuilding command-line arguments and parsing them with clap, and the
  referee's code, which the threat model (SPEC §8.0) bounds, is spread
  through the binary with the rest.
- Decision:
  - A Cargo workspace under `crates/` with seven members:
    `evalsig-core` (types, metrics, validity checks; no I/O),
    `evalsig-store` (objects, ledgers, canonicalization, hashing, sealing),
    `evalsig-adapters` (ingestion),
    `evalsig-project` (the `.evalsig/` project: data in force, settings
    resolution, provenance and ledger lines, import, the analyses each
    command computes, validation against the published JSON Schemas, and
    the loopback HTTP primitives both servers share),
    `evalsig-referee` (the referee's home, approval, the gate, the Ladder,
    sealing and `gate serve`),
    `evalsig-dashboard` (screens, HTML and SVG, the dashboard server) and
    `evalsig-cli` (binary `evalsig`: argument parsing, rendering, agent
    mode and wiring).
  - Dependencies point inwards: `core` depends on no other member; `store`
    and `adapters` may depend on `core`; `project` on `core`, `store` and
    `adapters`; `referee` and `dashboard` on `project` and below; `cli` on
    everything. `referee` and `dashboard` do not depend on each other.
  - Only `evalsig-cli` depends on `clap`. Library crates take typed
    requests, not command-line words.
  - Rust edition, toolchain pinning and MSRV are as in D-001, and so is the
    baseline dependency set: `serde`, `serde_json`, `toml` and `thiserror`
    in any member; `clap` and `anyhow` in the binary; the test-only crates.
- Alternatives: one application crate for project and referee together (not
  taken: the referee crate is the boundary a security review reads); keeping
  the layers as modules of the binary (not taken: the dashboard and a later
  MCP server need a library API).
- Consequences: the project and referee crates return `evalsig_project::Error`
  (a stable code, a message, a hint and an exit code), which the binary renders; the
  dashboard depends on `evalsig-project` rather than on the CLI's JSON alone.
  The published JSON Schemas move with the validation that embeds them, to
  `crates/evalsig-project/schema/`; the paths D-003, D-007, D-011 and D-012
  name are read accordingly.

## D-030 The recorded request of a saved result

- Status: Accepted
- Date: 2026-11-06
- Context: `provenance.request` says what `verify` recomputes (D-008, audit
  spec §5). It was the serialized command-line arguments of `report` and
  `compare`, so renaming a flag changed the saved form, and the library
  could not build a request without clap.
- Decision:
  - `request` is `{"command": "report" | "compare", "args": {...}}`, where
    `args` is a request type of `evalsig-project` with its own serde names.
    The command line converts its arguments into that type; the dashboard
    builds it directly.
  - `verify` recomputes `report` and `compare` results, the two commands
    that save results under `reports/`. Other commands do not save results
    and carry no `request`.
- Alternatives: saving results of `trend`, `agree` and `check` too (not
  taken for v1: none of them is a decision that a later reader has to
  reproduce, and each would add a saved file per run).
- Consequences: a new command that saves results adds a variant to the
  request type and to `verify` in the same change.

## D-031 Homebrew formula from the release archives

- Status: Accepted
- Date: 2026-11-07
- Context: D-020 left the Homebrew channel to a later task, which wrote a
  formula that built the release source with Rust. Each release then needed
  the source tarball's checksum copied into a tap by hand, and every user
  compiled evalsig. The maintainer's tap, `kechol/homebrew-tap`, already
  receives formulas that cargo-dist writes for other tools.
- Decision:
  - cargo-dist also builds a Homebrew installer: a formula named `evalsig`
    that installs the release archive for the user's platform (aarch64 and
    x86_64 macOS and Linux), checked against the archive's SHA-256.
  - The release workflow's `publish-homebrew-formula` job commits the
    formula to `kechol/homebrew-tap` after the GitHub Release is published,
    and skips pre-releases. It authenticates with the repository secret
    `HOMEBREW_TAP_TOKEN`, a token that may write that repository's contents
    and nothing else.
  - Users install with `brew install kechol/tap/evalsig`. The repository
    keeps no formula of its own.
- Alternatives: the hand-kept source formula (not taken: a manual step per
  release and a Rust toolchain on every user's machine); homebrew-core (not
  taken for 0.1.0: it expects an established project).
- Consequences: if the job fails, the GitHub Release stays published and
  the tap keeps the previous version until the job is rerun. Whoever holds
  the token can change the tap's formulas, so it is rotated like any other
  credential.

## D-032 Charts drawn as SVG by the dashboard crate

- Status: Accepted
- Date: 2026-11-07
- Context: The dashboard and `--format html` show signal-detection results that
  read best as pictures: the equal-variance model behind d′ and c, runs in ROC
  space, agreement by rater, performance by k. D-019 keeps the pages free of
  JavaScript under `default-src 'none'`, loading nothing from outside, and every
  drawn value must stay checkable against the CLI's JSON.
- Decision:
  - Charts are SVG strings that `evalsig-dashboard::svg` writes on the server:
    interval plots, curves on a square (ROC, z-ROC, calibration), the
    equal-variance model, ROC space with each run's curve of constant d′, bars
    and lines with interval bands, on shared axes, ticks and legends.
  - Each drawn value sits in a `data-*` attribute and a `<title>` exactly as the
    JSON writes it; model curves are drawn from the reported d′ and c with the
    normal distribution functions of `evalsig_core::numeric`.
  - Marks take their colours from the stylesheet's tokens through classes
    (series, tones), so charts follow the light or dark appearance.
- Alternatives: JavaScript libraries such as Chart.js, ECharts or Vega-Lite
  (not taken: they need scripts, which D-019 and the policy forbid); `plotters`
  with its SVG backend (not taken: it writes colours into each element and
  cannot attach the reported values to what it draws); `poloto` (not taken: no
  areas, bands or bars).
- Consequences: a new kind of chart is code in `svg.rs` with a test that its
  attributes carry the reported values; no chart dependency is added.

## D-033 The dashboard's own text in English or Japanese

- Status: Accepted
- Date: 2026-11-07
- Context: D-002 makes English the language of documentation and CLI output.
  The dashboard is read by people, among them the maintainer's Japanese-speaking
  users, and its own text (screen names, headings, buttons, chart labels) is
  neither documentation nor output another program parses.
- Decision:
  - `[dashboard] language` (`en` by default, or `ja`) chooses the language of
    the dashboard's own text. Each piece of it is one variant of
    `evalsig_dashboard::i18n::T` with its English and Japanese text side by
    side, so a missing translation does not compile.
  - Verdicts, warnings, errors and their hints, the glossary's meanings in "How
    to read", metric names and every value stay the library's English, as D-002
    has them, in both languages; `--format html` stays English like all CLI
    output.
  - The setting changes no result, so it is not serialized into the stored
    configuration and leaves configuration hashes and provenance unchanged.
  - The Japanese follows the terms of `README.ja.md`; run and item keep their
    English names there and here.
- Alternatives: translating verdicts and warnings too (not taken: agents and
  scripts read them, and two texts per verdict would drift); a language flag
  on `evalsig dashboard` (not taken: a project's readers share one setting,
  and the page reads its configuration on every request).
- Consequences: new dashboard text is a new variant with both texts; another
  language adds a column to every variant.

## D-034 Gate difference rules read the oriented difference

- Status: Accepted
- Date: 2026-11-07
- Context: The gate had three difference rules. `delta_lower_bound_at_least` and
  `delta_upper_bound_at_most` read candidate − baseline on the metric's own scale, so
  a lower-is-better metric needed the second rule and the sign of its margin, while
  `not_worse_than` read the difference oriented so that positive is better. Reading
  one rule in the other's orientation let a candidate with more false alarms pass. The
  bounds were inclusive, so a candidate identical to the baseline, whose bootstrap
  interval is [0, 0], passed `delta_lower_bound_at_least = 0`. Approval's power assumed
  the candidate better by exactly `min_effect`, where the point condition alone caps the
  pass probability near one half, and it fixed accuracy, the holdout size and one side.
- Decision:
  - Two difference rules, both on the difference oriented so that positive is better
    (as `compare` reports it) and both strict: `better_by = m` passes when the lower
    bound is above m and the point reaches `min_effect`; `not_worse_than = m` passes
    when the lower bound is above −m. m is a margin of at least 0; `min_effect` belongs
    to `better_by` only.
  - A difference condition may state `expected_effect`, the difference its hypothesis
    predicts. Approval reports, for each such condition, the probability that it passes
    at that effect, on the metric's denominator in the holdout, at the condition's own
    level and sidedness under the budget, with the Ladder's discordance ψ̂, and warns
    (`low_power`) below 0.8.
  - The Ladder's value for a difference rule is the oriented difference to the
    baseline.
- Alternatives: keeping inclusive bounds and refusing zero-width intervals (not
  taken: a candidate right on every item where the baseline was wrong also has a
  zero-width interval, at 1, and must pass); keeping the raw-scale rules beside an
  oriented one (not taken: two orientations for one comparison is how the defect
  arose); power at `min_effect` (not taken: about one half by construction).
- Consequences: gate.toml files written for the earlier rules are refused by the
  schema; a margin is written as a positive number.

## D-035 Each run's item inputs come from its own imports

- Status: Accepted
- Date: 2026-11-07
- Context: `compare` excludes items whose input changed between the arms, by the
  items' `input_ref`. The data in force layers every import into one item record per
  `item_id`, the latest import's, so both arms read the same hash and the check never
  fired: a dataset revision that kept the item IDs was compared item by item as if
  unchanged.
- Decision:
  - The data in force also keeps, per run, the `input_ref` that the snapshots holding
    the run's judgments declare for each item, the latest import winning.
  - A comparison reads each arm's input hash there; a run whose imports declare none
    for an item takes the layered item record, as before.
  - `records/1` does not change: the item record stays the place an input hash is
    declared.
- Alternatives: an `input_ref` on every judgment (not taken: a change to `records/1`
  and a hash repeated per replicate for a property of the item as imported); one item
  record per run in the layering (not taken: everything else reads one item per ID).
- Consequences: two runs imported with different item records for the same IDs now
  give `input_changed` and are compared on the unchanged items only.

## D-036 Numerical dependencies, with a bisected Beta quantile for large shapes

- Status: Accepted
- Date: 2026-11-07
- Supersedes: D-009
- Context: D-009 kept no fallback for statrs's Beta inverse because it matched the
  reference values. For a total shape above about 7.26e6 its Newton iteration
  overflows and never returns, so an exact bound on millions of judgments (a
  `missing_rate` condition in `check` or `gate`) hung.
- Decision:
  - `statrs` (default features off, `std` only) provides the normal and Beta
    distribution functions. Up to a total shape of 1e5, `beta_ppf` uses statrs's
    inverse, which matches the exact and Jeffreys interval reference values within
    1e-9; above it, `beta_ppf` bisects statrs's distribution function to the
    resolution of a double, at most 200 halvings.
  - `rand_chacha` (default features off) provides ChaCha20, seeded with
    `seed_from_u64` from the configured or recorded seed. Uniform integers use
    Lemire's unbiased method and uniform doubles take 53 bits, both written in
    evalsig, so no distribution code of `rand` is involved.
  - Sums run in a fixed iteration order with Neumaier compensation, so results do
    not depend on threads or map order.
  - A test pins the first ChaCha20 outputs and a derived mean for a fixed seed; it
    passes on macOS and on Linux.
- Alternatives: a normal approximation for large shapes (not taken: Beta(1, n), the
  bound at no events, is far from normal); writing the special functions ourselves
  (not taken: statrs meets the tolerance and is widely used).
- Consequences: upgrading `statrs` or `rand_chacha` requires the reference and pinned
  tests, including the large-shape quantile test, to pass unchanged.

## D-037 A rater's adjudication is a label of its own

- Status: Accepted
- Date: 2026-11-07
- Context: A label was identified by (item, rater, task, label replicate). A rater who
  adjudicated a split under the same rater ID as their vote therefore wrote a revision
  of the vote: at version 1 the import stopped on a duplicate key, at version 2 the
  adjudication silently replaced the vote, so `majority` and `soft` lost a voter and
  read one rater's vote as the reference.
- Decision:
  - A label's identity also says whether it is an adjudication (`kind =
    "adjudication"`). An adjudication stands beside the same rater's vote; a
    `revision` remains a revision of the rater's label.
  - The snapshot's primary key orders a vote before an adjudication of the same
    (item, rater, task, label replicate), before `version`, so data without such pairs
    keep their order and hashes.
- Alternatives: refusing an adjudication under a voter's rater ID (not taken: the
  adjudicator is often one of the raters, and a second ID per person misstates who
  labelled).
- Consequences: a records file holding a rater's vote and adjudication of one item now
  imports, and only `single` reads the adjudication.

## D-038 A gate adjusts the raters of `human_parity` by Holm

- Status: Accepted
- Date: 2026-11-08
- Context: D-017 adjusts the raters of the alternative annotator test by
  Benjamini-Yekutieli at q, after the paper. A gate ran the same procedure at α/B and
  counted it as a test at that level, but Benjamini-Yekutieli bounds the expected share
  of raters wrongly beaten, not the chance of a false `human_parity`: with 20 raters at
  the margin that chance is 0.112 at q = 0.05, and above q from about 11 raters.
- Decision:
  - In a gate, the p-values of the raters tested together are adjusted by Holm at the
    submission's level α/B. A false `human_parity` needs at least one rater wrongly
    beaten, and Holm bounds that chance at α/B under any dependence.
  - `agree --parity` keeps Benjamini-Yekutieli at q = 0.05 (D-017). The output names
    the adjustment (`adjustment`).
- Alternatives: Bonferroni (not taken: Holm has the same guarantee and wins more
  often); keeping Benjamini-Yekutieli and stating the weaker guarantee (not taken: the
  gate's promise that an ineffective candidate passes with probability at most α would
  not hold for parity conditions); a test of ω itself, such as a binomial test on the
  winning share (not taken: raters share items, so their wins are not independent).
- Consequences: a gate's `human_parity` is harder to pass than `agree --parity` on the
  same data when there are many raters, and a pass carries the gate's guarantee.

## D-039 The core library offers what the commands compute

- Status: Accepted
- Date: 2026-11-08
- Context: `evalsig-core` carried tested functions that no command calls, beside the
  pipeline that does the same work: a chooser of interval methods and the
  `[stats] interval` setting it read, which nothing applied; `score.rs`, whose
  `rate_at_least` differs from the pipeline's; `missing::mean_with_missing`;
  `paired::paired_mean_difference`; `paired::mcnemar`; `power::gate_pass_probability`;
  and `sdt::Confusion`'s own counting and rates. Their reference and unit tests
  certified the copy that ships nowhere, and a setting that changes nothing misleads.
- Decision:
  - A public function of `evalsig-core` is one a command uses. The unused twins and the
    `[stats] interval` setting are removed; a configuration that sets `interval` is
    refused as an unknown key.
  - Reference cases are checked through the functions the commands call (for example
    score metrics through `report::build`).
- Alternatives: wiring the chooser and the setting into `report` (not taken: the
  methods each metric takes are fixed by metrics spec §1.1, and the other choices would
  let a display interval degenerate where the spec avoids it); keeping the twins as
  library extras (not taken: they drift from the pipeline unseen).
- Consequences: a library user who called the removed functions computes the same
  quantities through `report`, `compare` and `power`.


## D-040 Errors say what failed and what to run next

- Status: Accepted
- Date: 2026-11-09
- Context: An agent recovers from a failed command only through the error it reads. Every
  usage error carried the code `usage`, whether a run was unknown, an argument missing
  or a command misspelt; a parse error kept only the first line of the parser's message,
  which drops the missing argument's name and the suggested spelling; and the hint that
  named a command wrapped it in backticks, which agent mode withholds as a possible
  quote (D-018), so an agent saw no hint at all. A bare `evalsig` answered agent mode
  with that error.
- Decision:
  - An error is `{"error": {"code", "message", "hint", "next"}}`. `next` is a list of
    `command` and `why`, as in the envelope, and is always present. Its commands are
    built from evalsig's own vocabulary (command and option names, placeholders such as
    `<run>`), never from the input, so agent mode prints them in full.
  - A common failure has a code of its own, still with exit 64: `unknown_command`,
    `missing_command`, `unknown_argument`, `missing_argument`, `invalid_value`,
    `conflicting_arguments`, `unknown_run`, `unknown_item`, `unknown_split`,
    `unknown_metric`, `unknown_period`, `unknown_name`, `already_exists` and
    `confirmation_required`. Other usage errors keep `usage`.
  - A parse error's message keeps every paragraph of the parser's message except the
    usage line.
  - Hints and messages that evalsig writes name commands without backticks, and a
    command to run goes in `next`; agent mode still withholds a message or hint that
    quotes.
  - In agent mode or with `--format json`, a bare `evalsig` prints the command index
    (`evalsig.index/1`): each command's name and summary, with `next` pointing at
    `help --json <command>`, `recipes` and `capabilities`. It exits 0.
  - `exclude` and `split` take `--dry-run`, which reports what would be recorded without
    writing the ledger.
- Alternatives: keeping `usage` and adding the specific reason in a second field (not
  taken: two codes for one error, and 0.1.0 has just been released); printing quoted
  values in agent mode (not taken: they can come from the evaluated data, D-018).
- Consequences: a consumer that matched `code == "usage"` for these failures matches
  the new codes or the exit code 64, which is unchanged. The error object's JSON Schema
  gains `next`.

## D-041 Expected cost per stratum and in `compare`

- Status: Accepted
- Date: 2026-11-11
- Context: Whether a revision of a leveled judge is worth adopting is a question of expected
  cost, not accuracy: fewer misses can mean more items sent to people. `threshold` computed
  expected cost (EC) for one run with one prevalence; `compare` did not compare it, and
  production prevalence often differs by stratum while test sets are balanced.
- Decision:
  - `[cost]` gains `by` (a stratum key) and `[cost.strata."<value>"]` with optional
    `prevalence`, `matrix` (same shape as the default) and `mix` (the stratum's production
    share). A stratum without an override uses the defaults.
  - EC per stratum is §4's formula on that stratum's items; each item contributes the share of
    its judgments at each level. Overall EC is Σ m_g EC_g / Σ m_g over the strata present.
    m_g is the configured `mix` when every stratum present has one, otherwise the stratum's
    weighted item count, and the output then says `mix: observed` and warns.
  - `expected_cost` is a metric of `compare` (lower is better) when `[cost]` is set and both
    runs' judgments are levels 0 … k−1: a paired cluster bootstrap on the common items, the
    §7.1 verdict with `min_effect.expected_cost`, and one row per stratum. `report` gives EC
    overall and per stratum with a cluster-bootstrap interval.
  - Items with no level judgment are left out of EC (no missing bounds, as for precision).
- Alternatives: the requested `weight` multiplier on the positive row (not taken: a per-stratum
  matrix says the same with one shape); a fixed observed mix (not taken: a balanced test set
  misstates production).
- Consequences: `compare` lists one more metric when `[cost]` is set; the default primary metric
  is unchanged. Unpaired and sequential comparisons do not include EC.

## D-042 Paired signal-detection deltas in `compare`

- Status: Accepted
- Date: 2026-11-11
- Context: When a revision lowers the miss rate, the question is whether the judge separates
  the classes better or only says "positive" more often. `report` gives d′ and c per run;
  the before/after question needs paired intervals on the same items.
- Decision:
  - `compare --sdt` adds `sdt`: ΔH, ΔF, Δd′ and Δc (candidate − baseline as measured) from
    the same weighted counts and class scaling as `report`, with paired cluster-bootstrap
    intervals from one set of resamples, and the equal-variance assumption named.
  - `sdt.change` classifies from the intervals: `discrimination_improved` (Δd′ above 0),
    `discrimination_worsened` (below 0), `threshold_moved` (Δd′ contains 0, Δc excludes it),
    else `undetermined`; `insufficient_data` when a cell (hits, misses, false alarms,
    correct rejections) of either arm is below `--min-events` (default 5, T-140) or there
    are fewer than five clusters. It is not a `verdict.code`; the verdict set is unchanged.
  - ΔAUC is not included: a paired AUC needs its own interval method.
- Alternatives: new verdict codes (not taken: D-008 would raise the schema version).
- Consequences: `--sdt` needs decisions and a reference; it is refused otherwise.

## D-045 Compressed sources and filters in `evalsig.mapping/1`

- Status: Superseded by D-046
- Date: 2026-11-11
- Context: A common run log is gzipped JSONL with replicates nested in each line and a list
  of rule results of which only the applicable ones are features. `mapping/1` could not
  read it without a converter (D-011 prefers a declared mapping).
- Decision:
  - Every reader of input files decompresses gzip and Zstandard, recognised by their first
    bytes, within the size limit applied to the decompressed bytes. `flate2` (MIT OR
    Apache-2.0, already in the tree through `zip`, with the pure-Rust `zlib-rs` backend)
    becomes a direct dependency of `evalsig-adapters`; Zstandard stays with `ruzstd`.
  - `judgment.explode.filter` and `judgment.features.filter` keep the elements a condition
    accepts; `judgment.features.fired_if` sets `fired` from a condition, in place of
    `fired`. Replicates a log marks as copies are dropped with the filter. All three are
    optional keys, so existing mappings keep their meaning and hash.
- Alternatives: a `source.compression` setting (not taken: the bytes say it); a
  `replicate.copy_of` key (not taken: the filter covers a marked copy, and an unmarked one
  should keep raising `identical_replicates`).
- Consequences: a compressed file's recorded hash is that of its decompressed bytes.

## D-046 Compressed sources and `explode.filter` in `evalsig.mapping/1`

- Status: Accepted
- Date: 2026-11-11
- Supersedes: D-045
- Context: D-045 added `features.filter` and `features.fired_if`, but the path subset
  already filters elements (`[?(@.applies == true)]`) and `value_map` with `default` turns
  a value into `fired`; two ways to say one thing would be kept forever.
- Decision: compressed sources as in D-045. `judgment.explode.filter` keeps the elements or
  keys a condition accepts; it is new because a split by keys has no element a path filter
  could test. Features are filtered and set with the existing path and expression syntax.
- Consequences: a mapping written for D-045's two feature keys is refused as unknown keys.

## D-047 Joining row-based label files by a key

- Status: Accepted
- Date: 2026-11-11
- Context: A JSONL or CSV label file joins as an array, and a path filter compares with a
  fixed literal, so labels could not be matched to rows by a computed key.
- Decision: `label.join.key = { label = "<template>", row = "<template>" }`, in place of
  `join.path`, matches each label row whose `label` template (over `@`) equals the row's
  `row` template (over `$`), such as `"{@.task}:{@.unit_id}"` and `"{$.task}:{$.id}"`.
  Every matching label row gives a label, so several raters' rows are kept.
- Alternatives: templates inside filter literals (not taken: a composite key needs one
  filter per field, and filters do not chain on arrays).

## D-048 Attribution of errors

- Status: Accepted
- Date: 2026-11-11
- Context: Which causes the remaining errors have, and how much revising the rules can still
  fix, is computed outside evalsig, counting overlapping causes twice and without intervals.
- Decision: `records/1` gains an `attribution` record (`item_id`, `run_id`, `task`,
  `rater_id`, `causes`: user strings), an annotation made after the fact with its own author,
  not a field of the judgment. `report --decompose misses|false_alarms` assigns each error to
  its first cause in `[attribution] order` (unlisted causes after, by name) and gives shares
  and `--ceiling` rates with cluster-bootstrap intervals; unattributed errors are a cause.
- Consequences: readers of `records/1` meet one more record kind; writers gain a function.

## D-049 Feature direction and lineage

- Status: Accepted
- Date: 2026-11-11
- Context: A rule written to point one way can turn out informative the other way, and rules
  are renamed and split between runs, so their LRs could not be followed across revisions.
- Decision: the `run` record gains optional `features`: per feature ID, `expected`
  (`positive` or `negative`, the class it should point to) and `lineage` (a stable name; a
  feature without one is its own lineage). `report --features` adds `direction`
  (`as_expected`, `reversed`, `uninformative`, `insufficient`) from the adjusted interval;
  `compare --features` gives each lineage's LR in both runs, a lineage firing when any member
  fires in a judgment. Declared per run, since rules change between runs.

## D-050 Shareable bundles of a project's records

- Status: Accepted
- Date: 2026-11-11
- Context: Collaborators reproduce each other's reports from raw run logs. A canonical export
  imported as one file reproduces the snapshot and `verify` passes (T-150).
- Decision: `export --to bundle` writes `records.jsonl` (the records in force, canonical, in
  primary-key order: the snapshot's bytes; no content records) and `manifest.json` (its
  BLAKE3, the configuration hash, tool and spec version). `--hash-ids` replaces item IDs,
  clusters and strata values with a keyed BLAKE3 under `EVALSIG_BUNDLE_KEY`, which is never
  written, and drops `attrs`, label reasons and missing details. `import --from-bundle` refuses
  a records file whose hash differs from the manifest's.
- Consequences: a hashed bundle's reports verify against the bundle, not the original.

## D-051 Metrics per unit of several items

- Status: Accepted
- Date: 2026-11-11
- Context: A document judged section by section counts a miss in one section and a false
  alarm in another, although the document as a whole was caught.
- Decision: `report --unit cluster --aggregate any` treats the items of a cluster as one unit:
  its reference and judgment are the largest of its items' reference shares and judged shares
  (for 0/1 values, positive or flagged if any item is). A unit with an item without a judgment
  is missing, not negative. H, F and accuracy over units get unit-bootstrap intervals.
