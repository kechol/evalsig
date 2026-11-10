# Storage and audit trail

evalsig keeps what it needs for a third party to check three things later:

1. **Reproduction.** A reported number recomputes from the recorded data,
   configuration and tool version.
2. **Tampering.** Data, labels and gate results were not silently rewritten.
3. **Procedure.** The gate conditions were fixed before the results, and the
   record shows how often a holdout was queried and who excluded or purged
   what.

This page describes what the code stores and checks. The referee's threat
model and deployment are in [referee.md](referee.md); the commands and their
options are in [cli.md](cli.md). Terms follow [glossary.md](glossary.md) and
decisions are cited as `D-NNN` ([decisions.md](decisions.md)).

## 1. Source of truth

| Layer | Contents | Property |
|---|---|---|
| Objects | Snapshots, provenance, configurations and content bodies, each a file named by its hash | Immutable; the only operation that removes one is `purge` |
| Ledgers | `ledger.jsonl`: an agent-side ledger and a referee ledger (section 4) | Append-only; each line carries the hash of the previous line |

Every command reads the data in force from these two layers; there is no
derived database (D-028). Canonical JSON can be defined independently of any
library, so tools in other languages verify the same hashes.

### Canonical bytes (D-005)

- Every record, ledger line, configuration and provenance object is turned
  into canonical bytes: RFC 8785 (JSON Canonicalization Scheme), with every
  string and object key first normalized to Unicode NFC. Keys sort by UTF-16
  code units; numbers use the ECMAScript shortest form.
- Input is rejected, not repaired, when an object has duplicate keys (also
  after NFC), when an integer lies outside ±(2^53 − 1), or when it holds NaN,
  an infinity or an unpaired surrogate. Scores that are NaN or infinite
  become `missing` (`invalid_output`) at import.
- Ledger summaries hold no floating-point numbers; metric values there are
  rounded strings (D-007). Numbers in command results are rounded to 10
  decimal places before they are hashed.
- `tests/fixtures/canonicalization-vectors.json` lists inputs with their
  canonical bytes and their BLAKE3 and SHA-256 hashes. The Rust encoder
  (`crates/evalsig-store/src/canonical.rs`) and the Python reference
  (`scripts/evalsig_canon.py`) must produce the same values for every vector.

### Hashes and objects

- A reference is `blake3:<64 hex>` or `sha256:<64 hex>`. evalsig writes
  BLAKE3 and reads both, so a third party can check a ledger with the Python
  standard library alone. The object store looks up `blake3:` names only.
- An object is stored zstd-compressed at
  `.evalsig/objects/blake3/<2 hex>/<62 hex>.zst`, named by the BLAKE3 hash of
  its uncompressed bytes (D-010). Writing goes to a temporary file that is
  renamed into place, and an existing object is never rewritten, so importing
  the same data twice stores it once.
- Reading an object checks that its content hashes to its name; a mismatch is
  reported as corruption.
- The hash of a snapshot file is BLAKE3 over its canonical lines joined with
  LF, without a trailing newline.
- A set of items (a split, the items a comparison used) is named by the
  BLAKE3 reference of the canonical JSON array of its sorted item IDs (D-022).

## 2. Layout

The agent's project, `.evalsig/`:

```text
config.toml      project configuration
ledger.jsonl     agent-side ledger
objects/         snapshots, provenance, stored configurations, content bodies
reports/         saved results, named <output hash>.json
splits/          dev and validation splits
drafts/          preregistration drafts
```

The referee's home (D-014), under another authority than the agent's:

```text
identity.txt     age secret key (mode 0600)
recipient.txt    the matching age recipient
ledger.jsonl     referee ledger
holdouts/        the item IDs of each holdout, named by its item-set hash
sealed/          sealed holdout labels, <holdout hash>.age
approved/        approved conditions, resolved in full, <hash>.toml
results/         full gate results
exclusions/      item sets the referee excluded
state/           lock file, per-holdout label hashes and the Ladder's best candidate
```

The referee ledger belongs where the agent cannot write (see
[referee.md](referee.md)). Appends hold an exclusive file lock, so concurrent
gates serialize.

## 3. Snapshots and provenance

An import writes two objects.

- The **snapshot** is the set of canonical records (`records/1`, D-003) of
  the import: run, item, label, judgment, preference and content records, one
  per line, in primary-key order (by kind in that order, then by the kind's
  key fields, strings by code point). Its name is the file hash of those
  lines, so it depends on the data alone: importing the same records in
  another order gives the same snapshot.
- The **provenance** object (`evalsig.provenance/1`) says where the snapshot
  came from: the snapshot's hash, each source file's path and hash, the
  adapter and its version, the hash of the `mapping.toml` or converter that
  produced the records, the hashes of the mapping decisions, the evalsig
  version, the import time, the actor, and the redactions (section 6).

The `import` ledger line lists the source hashes and the mapping hash as
inputs and the snapshot and provenance hashes as outputs. The **snapshots in
force** are those named by `import` lines, each replaced by its replacement
where a `purge`, `migrate` or `exclude` line (sections 6 and 8) superseded
it; importing a superseded snapshot again brings in its replacement.

## 4. Ledgers

### Line format (D-007)

```json
{"seq": 12, "ts": "2026-11-03T10:00:00Z", "actor": "user:alice",
 "kind": "report", "ledger": "agent",
 "inputs": ["blake3:…snapshot", "blake3:…config"],
 "outputs": ["blake3:…output"],
 "summary": {"run": "candidate-v2", "seed": 7},
 "tool": "evalsig 0.1.0",
 "prev": "blake3:…previous line", "hash": "blake3:…this line"}
```

- `seq` counts from 0. `prev` is the previous line's `hash`; the first line
  has `genesis`. `ledger` is `agent` or `referee`.
- `hash` is the hash of the canonical bytes of the line without `hash`, with
  the algorithm the line's own `hash` names (BLAKE3 when written by evalsig).
- `actor` is read from `EVALSIG_ACTOR`, else `agent:<AI_AGENT>`, else
  `user:<USER>`. It is self-reported and not authenticated.
- An append takes an exclusive lock, reads the last line, writes one line and
  syncs. A ledger whose last line is incomplete is refused until repaired.
  Each ledger accepts only its own kinds; the other kinds are refused at
  append and reported by verification.

### The two ledgers

| Ledger | Held by | Kinds a command writes |
|---|---|---|
| Agent-side | the agent's project | `import`, `mapping_decision`, `migrate`, `split` (dev and validation), `exclude`, `purge`, `preregister_draft`, `report`, `compare`, `verify` |
| Referee | the referee's home | `split` (the holdout), `seal`, `label_revision`, `preregister_approve`, `holdout_access`, `gate`, `exclude` (holdout items) |

The agent can rewrite its own ledger, so it is evidence, not proof. The
referee copies the head of the agent-side ledger into each `gate` line when
the submission names it; only the referee ledger and `results/` back a gate
result. `adjudication` and `anchor` are accepted by the referee ledger, and
`label_revision` and `adjudication` by the agent-side ledger, but no command
writes them today.

### Kinds

| Kind | Written when | Main content |
|---|---|---|
| `import` | data is imported | inputs: source and mapping hashes; outputs: snapshot and provenance |
| `mapping_decision` | `map decide` records an answer | input and output: the mapping's hash before and after; the decision ID, the choice, who decided (self-reported) |
| `migrate` | snapshots are rewritten in the current format | `summary.replaced`: old and new snapshot and provenance hashes, old and new format |
| `split` | a split is made (agent: dev and validation; referee: the holdout) | method, seed, item-set hashes; the agent's line outputs the split's name |
| `exclude` | items are excluded | reason code, detail, item count; input: the item-set hash; agent-side also `summary.replaced` |
| `purge` | a body is purged | reason, `summary.replaced`, counts, deleted object and report names |
| `preregister_draft` | a draft of conditions is saved | output: the draft's hash; the draft's name and condition count |
| `preregister_approve` | the referee approves a draft | inputs: the draft, the holdout and the seal line; output: the hash of the resolved conditions; budget, per-submission level, power at the hypothesis |
| `seal` | holdout labels are sealed | output: hash of the sealed file; holdout, label and item counts, whether it is a reseal |
| `label_revision` | a reseal finds changed holdout labels | the hashes of the new label lines only; no value |
| `holdout_access` | a submission passes the coverage and budget checks | the holdout, the approval and the candidate's hash; one line is one use of the budget |
| `gate` | a submission is judged | approval and candidate hashes, verdict, per-condition pass or fail, error code, the agent-side ledger head if named, whether the seal was void, label-dependent warnings (recorded here and not returned) |
| `report`, `compare` | an analysis runs (a `compare --scratch` writes no line) | inputs: snapshots and configuration; output: the result's hash |
| `verify` | a result is recomputed | input: the saved result's hash; output: the recomputed hash; whether they match |

### Detecting changes: the chain and anchors

`evalsig ledger verify` checks every line: `seq` equals its position, `prev`
equals the previous `hash`, `hash` matches the content, and, with a side
named, the line belongs to that ledger and its kind is allowed there. It
reports each problem with its line number and exits with the `blocked`
verdict when there is one.

A chain shows that a line in the middle was edited. It cannot show that the
tail was deleted or that the whole chain was rebuilt. For that, record the
head hash outside the ledger: `evalsig ledger anchor` prints the head hash and
its `seq`; commit it to git, print it in a CI log or paste it in a pull
request. Later, `evalsig ledger verify --anchor <hash>` fails when no line
has that hash. Anchoring is a manual step; no command writes an `anchor`
line.

Lines are not signed. A holder of the file can rebuild a whole chain, and only
an anchor taken earlier exposes it.

Outside evalsig, `scripts/verify_store.py <.evalsig dir | ledger.jsonl>`
checks ledger chains, object names, saved results and their stored
configurations and snapshots with an independent implementation of the
hashing rules. `scripts/cross_verify.sh` builds stores with the binary and
confirms that the verifier rejects a rewritten ledger line and a rewritten
report value.

## 5. Provenance in output and verification

Every command's JSON envelope (D-008) has a `provenance` object. For `report`
and `compare` it holds what the result was computed from:

| Field | Meaning |
|---|---|
| `snapshots` | The snapshots in force that the analysis read |
| `config` | Hash of the stored configuration: the project configuration with every default filled in, as canonical JSON (D-004), kept as an object |
| `seed` | The resampling seed |
| `request` | The command and arguments, so the computation can be repeated |
| `output` | BLAKE3 of the canonical `results` |
| `tool`, `spec_version` | The evalsig version and the metric definitions (`metrics/2`) it follows |
| `ledger`, `ledger_seq` | Which ledger recorded the command, and the line |

The envelope is saved as `.evalsig/reports/<output hash>.json` before the
ledger line names that output, so a recorded output always exists. Other
commands fill in the fields they have (an import names its snapshot; the
referee's commands say `referee`).

`evalsig verify <report>` reads a saved result, loads the snapshots and the
configuration it names from the object store (which checks each object against
its name), repeats `request` and compares the recomputed `results` with the
saved ones, leaf by leaf. The answer lists every differing path with both
values, the saved and recomputed output hashes and the recorded one. A
mismatch exits with `fail`; an unreadable or incomplete result exits as
rejected. It then appends a `verify` line. Specifics:

- Numbers are rounded to 10 decimal places before hashing so a one-bit
  difference between CPUs does not fail a check.
- A different `tool` or `spec_version` is reported as a warning; a formula
  change ships with a CHANGELOG entry and a new `spec_version`.
- For a `compare` on a validation split, `validation_uses` is recounted from
  the ledger lines before the one that recorded the result. A `--scratch`
  comparison is in no line, so its count is taken as saved, with a warning.
- Verification stands on the objects and the configuration, not on the
  ledger: it does not check the chain. Run `ledger verify` for that.

## 6. Content storage and purge

A record names the text of an input, output or grader rationale by a `*_ref`
hash, so the text can be removed without changing a snapshot or a ledger line.
`[store] content` in the project configuration chooses how bodies are kept
(D-024):

- `full` stores the body as an object under its own hash (the default).
- `hash` stores nothing. Reproduction and tamper detection still work; the
  text can be seen only while the source file remains.
- `redact` stores the body with every match of `redact_patterns` replaced by
  `[redacted]`, under the redacted bytes' own name. The `*_ref` in records is
  still the hash of the original body, and the provenance keeps the pair
  (original, stored). Matches in `item.attrs`, strata values, `label.reason`
  and `missing.detail` are replaced before the snapshot is written.

Import also checks for the common shapes of secrets (API keys, private keys)
and warns with `secret_in_content`; `--fail-on-secret` makes it stop.

`evalsig purge <ref> --reason <text> --yes` removes a body after the fact:

1. It finds the items whose records name the body and replaces their
   free-text fields (`attrs`, `reason`, `detail`, strata values) in every
   snapshot in force with the BLAKE3 reference of the value.
2. It writes the replacement snapshots and appends a `purge` line whose
   `summary.replaced` lists each (`from`, `to`, `provenance`). The ledger
   names the replacements before anything is deleted, so an interrupted purge
   leaves text behind and not data that cannot be loaded.
3. It deletes the body, its redacted copy if no other record names it, the
   superseded snapshots and the saved reports that name them. A `cache/`
   directory, which is outside the project layout and may hold a copy of the
   data, is deleted too (`stale_working_copy_removed`).

Data loading follows `purge` lines, and importing records that were purged is
refused. Superseded snapshots remain only as hashes in the ledger. Two
limits: a hash of a short or guessable value (a stratum such as `chat`, a
name) can be recovered by guessing, and a report computed before a purge
cannot be verified afterwards.

## 7. Sealing the holdout

Sealing keeps holdout reference labels out of the agent's environment while
letting the referee use them (D-013). The threat model and deployment forms
are in [referee.md](referee.md); this section covers the stored artifacts.

- **Flow.** The referee imports the labelled data into its own project, cuts
  the holdout with `split --holdout` (a `split` line in the referee ledger,
  named by the holdout's item-set hash) and hands the agent the remaining item
  IDs only. `seal` then encrypts the holdout's labels.
- **Refusal.** `seal` fails with `holdout_labels_in_agent` when any object in
  the agent project's store that reads as a snapshot, in force or superseded,
  holds a label of a holdout item. The agent's snapshots must never have held
  those labels.
- **Plaintext.** Every revision of every holdout label as a canonical record
  line, sorted and without duplicates, one per line.
- **Encryption.** The plaintext becomes an age v1 file to the referee's
  X25519 recipient, written to `sealed/<holdout hash>.age`. `age -d -i
  identity.txt` and `rage -d -i identity.txt` open it, so the file can be
  checked without evalsig.
- **Ledger.** The `seal` line outputs the BLAKE3 hash of the sealed file and
  lists the holdout, the label and item counts and whether this is a reseal.
  When a reseal finds label lines that were not in the earlier seal, a
  `label_revision` line records the hashes of those lines, never the values.
- **Key.** The secret key is `identity.txt` in the referee's home. Whoever can
  read it is the referee (D-014). `gate` records, and says in its output,
  that the seal is void when the home lies inside the agent's project
  directory or the identity is readable by group or others.
- **Budget.** A holdout's queries are the count of its `holdout_access` lines
  in the referee ledger. The count belongs to the holdout, so approving new
  conditions does not reset it.

The seal protects labels that exist independently of the judgments and that
the agent cannot regenerate, such as human judgments. It does not protect
labels derived from a model, or an agent that holds the key.

## 8. Migration and exclusion

Both replace snapshots the way `purge` does: a new snapshot and provenance are
written, a ledger line lists them in `summary.replaced`, and the old snapshot
is replaced in the snapshots in force.

**`migrate`** (D-003). Record lines carry no format version, so a snapshot's
format is recognized by shape: lines that all read as the current format are
current; otherwise the migration whose detector accepts every line names the
format. `migrate` rewrites each snapshot in an earlier format as a snapshot in
`records/1` through the chain of migrations, keeps the original and appends a
`migrate` line with the old and new snapshot and format for each. `--dry-run`
lists what would change and writes nothing. `records/1` is the only format, so
no migrations ship yet: `migrate` finds every snapshot current and writes
nothing, and a snapshot in an unrecognized format stops it with
`unknown_format`.

**`exclude`** (D-027). Leaves items out of every analysis, with a reason
code (`duplicate`, `invalid_input`, `out_of_scope`, `privacy`, `other`;
`other` needs a detail).

- On the agent's side, every snapshot in force that holds records of the
  items is replaced by one without them. The `exclude` line's input is the
  item-set hash, whose JSON array is kept in the object store. Earlier
  reports keep naming the superseded snapshots, which are not deleted, so
  `verify` still reproduces them. Records of the items that arrive later in a
  new snapshot come back, so repeat the exclusion after such an import.
- If a split was made within the referee's remaining items, the agent-side
  command refuses an item outside them (`holdout_item`): only the referee
  excludes holdout items.
- With a referee home, the referee excludes holdout items only. It writes
  `exclusions/<hash>.json` and an `exclude` line in its own ledger; from then
  on `seal`, `preregister approve` and `gate` leave those items out. Approvals
  pin the holdout's items, so `gate` refuses an approval made before the
  exclusion until the referee approves again; the budget stays with the seal.

## 9. What each part guarantees

| Part | Guarantees | Does not guarantee |
|---|---|---|
| Object store | An object's bytes match its name; any change, or a missing object, is reported when it is read or verified | That an object was never deleted before it was named anywhere; `purge` deletes by design |
| Canonical form | The same data gives the same bytes and hash in Rust and in Python | Anything about data that is rejected as not canonical |
| Snapshot | Identified by its data alone; unchanged after import | Where the data came from: provenance is a claim by the importer |
| Ledger chain | An edited line in the middle is found | A deleted tail or a rebuilt chain, without an anchor; who wrote a line (`actor` is self-reported); order in time (`ts` is the writer's clock) |
| Anchor | A chain taken apart or truncated later does not contain the anchored head | Anything about lines added after the anchor, or an anchor never taken |
| Agent-side ledger | A record of the agent's operations | Proof: the agent can rewrite it |
| Referee ledger | The same chain, in a place the agent cannot write, with every holdout query, gate result, seal and exclusion | Proof against a referee run with the agent's privileges |
| `verify` | A saved result recomputes from the stored snapshots and configuration, or the differences are listed | That the ledger is intact, or that an unpurged snapshot is the data the importer meant |
| Seal | Holdout labels are readable only with the referee's identity, and the sealed file is checkable with the standard age tools | Protection when the agent holds the key, for labels the agent can regenerate, or from a person who can read the plaintext label files before sealing |

Signatures on ledger lines are not implemented.
