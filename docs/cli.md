# CLI reference

This page is generated from `evalsig help --json` by
`crates/evalsig-cli/tests/cli_reference.rs`; a test fails when it differs from the
binary. Rewrite it with `EVALSIG_UPDATE_DOCS=1 cargo test -p evalsig-cli --test
cli_reference`. The terms are those of [the glossary](glossary.md); how agents
read the output is in [the agent interface](agent-interface.md).

## Global options

These apply to every command.

| Option | Description | Values | Default |
|---|---|---|---|
| `--format <FORMAT>` | Output format | `json`, `md`, `junit`, `text`, `jsonl`, `html` | `text` |
| `--project <DIR>` | Project root holding `.evalsig/` |  | `.` |
| `--agent` | Agent mode: one JSON envelope sized for an agent's context (D-018). Also on when AI_AGENT is set, the output is not a terminal and no --format is given |  |  |
| `--no-agent` | Never use agent mode (as EVALSIG_AGENT_MODE=0) |  |  |
| `--cursor <N>` | In agent mode: the first row of each list to show |  | `0` |
| `--include-content` | In agent mode: return text from the evaluated data in `untrusted_content` |  |  |

## Commands

### `evalsig agree`

Agreement between raters, of graders and a run with people, and on preferences

| Option | Description | Values | Default |
|---|---|---|---|
| `--task <TASK>` | The judgment task |  | `default` |
| `--estimand <ESTIMAND>` | The estimand: single_shot, majority@k, any@k or all@k (default from the configuration) |  |  |
| `--threshold <THRESHOLD>` | Decision threshold for score outputs; for score runs, also the share at or above it |  |  |
| `--labels-as-of <TIME>` | Use the labels as they were at this RFC 3339 time |  |  |
| `--reference <REFERENCE>` | How several raters' labels become the reference (default from the configuration) | `single`, `majority`, `soft`, `latent`, `human` |  |
| `--run <RUN>` | Also compare this run's decisions with each human rater, under the same rule as the raters among themselves |  |  |
| `--preference` | Agreement of raters on pairwise preferences, their position bias, and the Bradley-Terry ranking of the arms |  |  |

### `evalsig check`

Check one run against interval-based absolute criteria (not preregistered)

| Option | Description | Values | Default |
|---|---|---|---|
| `<FILE>` (required) | The conditions (a subset of gate.toml) |  |  |
| `--run <RUN>` (required) | The run to check |  |  |
| `--task <TASK>` | The judgment task |  | `default` |
| `--estimand <ESTIMAND>` | The estimand: single_shot, majority@k, any@k or all@k (default from the configuration) |  |  |
| `--threshold <THRESHOLD>` | Decision threshold for score outputs; for score runs, also the share at or above it |  |  |
| `--labels-as-of <TIME>` | Use the labels as they were at this RFC 3339 time |  |  |
| `--reference <REFERENCE>` | How several raters' labels become the reference (default from the configuration) | `single`, `majority`, `soft`, `latent`, `human` |  |
| `--allow-unexplained-missing` | Continue although some missing values have no explained reason |  |  |
| `--fail-on-secret` | Stop when content looks like a secret instead of warning |  |  |

### `evalsig compare`

Compare a baseline run with one or more candidates on their common items

| Option | Description | Values | Default |
|---|---|---|---|
| `<BASELINE>` (required) | The baseline run |  |  |
| `<CANDIDATES>` (required) | One or more candidate runs |  |  |
| `--metric <METRIC>` | The metric that decides the verdict (default: the first the runs support) |  |  |
| `--task <TASK>` | The judgment task |  | `default` |
| `--estimand <ESTIMAND>` | The estimand: single_shot, majority@k, any@k or all@k (default from the configuration) |  |  |
| `--threshold <THRESHOLD>` | Decision threshold for score outputs; for score runs, also the share at or above it |  |  |
| `--labels-as-of <TIME>` | Use the labels as they were at this RFC 3339 time |  |  |
| `--reference <REFERENCE>` | How several raters' labels become the reference (default from the configuration) | `single`, `majority`, `soft`, `latent`, `human` |  |
| `--items` | Include one row per paired item (`--format jsonl` prints only these rows) |  |  |
| `--scratch` | Do not record the comparison in the ledger |  |  |
| `--unpaired` | Compare the arms as independent samples whose items need not overlap (Newcombe, Welch or a bootstrap per arm) |  |  |
| `--sequential` | Also add the paired items one by one in a seeded order and report the confidence sequence, which stays valid however often it is inspected |  |  |
| `--split <SPLIT>` | Compare only the items of this part of the latest `evalsig split` | `dev`, `validation` |  |
| `--allow-unexplained-missing` | Continue although some missing values have no explained reason |  |  |
| `--fail-on-secret` | Stop when content looks like a secret instead of warning |  |  |
| `--allow-mixed-grader` | Continue although the compared arms used different grader versions |  |  |
| `--allow-input-change` | Keep items whose input differs between the compared arms |  |  |

### `evalsig init`

Create `.evalsig/` and a commented configuration template

| Option | Description | Values | Default |
|---|---|---|---|
| `--force` | Replace an existing configuration file with the template |  |  |

### `evalsig capabilities`

The analyses the imported data allow, and what to add for the others

| Option | Description | Values | Default |
|---|---|---|---|
| `--run <RUN>` | Only this run |  |  |

### `evalsig exclude`

Leave items out of every analysis, with a reason code (holdout items: the referee)

| Option | Description | Values | Default |
|---|---|---|---|
| `<ITEM>` (required) | The items to exclude |  |  |
| `--reason <REASON>` (required) |  | `duplicate`, `invalid_input`, `out_of_scope`, `privacy`, `other` |  |
| `--detail <DETAIL>` | Free text kept in the ledger; required with `--reason other` |  |  |
| `--referee <DIR>` | As the referee: exclude holdout items in this referee home (default: EVALSIG_REFEREE when set) |  |  |

### `evalsig dashboard`

Serve the local web dashboard on 127.0.0.1, behind a per-launch token (D-019)

| Option | Description | Values | Default |
|---|---|---|---|
| `--port <PORT>` | The port on 127.0.0.1 (0 picks a free one) |  | `7777` |

### `evalsig agent-guide`

Print the AGENTS.md snippet or the Claude Code permission template for evalsig

| Option | Description | Values | Default |
|---|---|---|---|
| `--snippet` | The snippet for AGENTS.md or CLAUDE.md, for agents without Skills |  |  |
| `--settings` | A `.claude/settings.json` permission template that guards against accidents |  |  |

One of `--snippet` or `--settings` is required.

### `evalsig explain`

What a term, metric, verdict, validity check or exit code means

| Option | Description | Values | Default |
|---|---|---|---|
| `<NAME>` (required) | A glossary identifier, verdict code, validity check or exit code |  |  |

### `evalsig export`

Write the data as Parquet, CSV or canonical JSONL for analysis elsewhere

| Option | Description | Values | Default |
|---|---|---|---|
| `--to <TO>` (required) | The file format | `parquet`, `csv`, `jsonl` |  |
| `--out <DIR>` (required) | The directory to write the files into (created if missing) |  |  |
| `--items <RUN>` | Also this run's per-item report rows, as the table items_<run> |  |  |
| `--task <TASK>` | The judgment task |  | `default` |
| `--estimand <ESTIMAND>` | The estimand: single_shot, majority@k, any@k or all@k (default from the configuration) |  |  |
| `--threshold <THRESHOLD>` | Decision threshold for score outputs; for score runs, also the share at or above it |  |  |
| `--labels-as-of <TIME>` | Use the labels as they were at this RFC 3339 time |  |  |
| `--reference <REFERENCE>` | How several raters' labels become the reference (default from the configuration) | `single`, `majority`, `soft`, `latent`, `human` |  |

### `evalsig gate`

As the referee: judge a candidate on the sealed holdout under approved conditions

| Option | Description | Values | Default |
|---|---|---|---|
| `--approved <HASH>` (required) | The approved conditions, by hash |  |  |
| `--candidate <RUN>` (required) | The candidate's run ID |  |  |
| `--records <FILE>` (required) | The candidate's records (records/1 JSONL): its run and judgments |  |  |
| `--mapping-hash <HASH>` | The hash of the mapping or converter that made the records, as the candidate's side reports it |  |  |
| `--agent-ledger <FILE>` | The agent-side ledger, whose head is copied into the gate line |  |  |
| `--referee <DIR>` | The referee's home (default: EVALSIG_REFEREE) |  |  |

### `evalsig gate serve`

As the referee: answer submissions on a Unix socket in the referee home and, with --http, on token HTTP at 127.0.0.1

| Option | Description | Values | Default |
|---|---|---|---|
| `--referee <DIR>` | The referee's home (default: EVALSIG_REFEREE) |  |  |
| `--http <PORT>` | Also answer token HTTP on 127.0.0.1 at this port (0 picks a free one) |  |  |

### `evalsig gate submit`

Submit a candidate to a running `gate serve` and print its answer

| Option | Description | Values | Default |
|---|---|---|---|
| `--approved <HASH>` (required) | The approved conditions, by hash |  |  |
| `--candidate <RUN>` (required) | The candidate's run ID |  |  |
| `--records <FILE>` (required) | The candidate's records (records/1 JSONL): its run and judgments |  |  |
| `--mapping-hash <HASH>` | The hash of the mapping or converter that made the records, as the candidate's side reports it |  |  |
| `--agent-ledger <FILE>` | The agent-side ledger, whose head is copied into the gate line |  |  |
| `--socket <PATH>` | The referee's Unix socket (`gate.sock` in its home) |  |  |
| `--url <URL>` | The referee's HTTP address, `http://127.0.0.1:<port>` |  |  |
| `--token-file <FILE>` | A file holding the bearer token for --url (default: EVALSIG_GATE_TOKEN) |  |  |

### `evalsig import`

Read records into a snapshot and record the import in the ledger

| Option | Description | Values | Default |
|---|---|---|---|
| `<FILES>` | Files to import into one snapshot |  |  |
| `--adapter <ADAPTER>` | Adapter; by default chosen from the file extension (.jsonl/.json, .csv) | `records`, `csv`, `inspect`, `promptfoo` |  |
| `--mapping <FILE>` | Map the files with a mapping file (`mapping/1`) |  |  |
| `--arg <NAME=VALUE>` | A value a mapping reads with `{ arg = "NAME" }`; repeat for several |  |  |
| `--from <NAME>` | Apply the `[[import]]` block of this name in the configuration; its `path` names the files unless files are given |  |  |
| `--as <ROW>` | What each CSV row is | `judgment`, `label` | `judgment` |
| `--system-version <SYSTEM_VERSION>` | Override `run.system.version` of the imported runs |  |  |
| `--grader <GRADER>` | Override `run.grader` as `<id>[@<version>]` |  |  |
| `--converter <PATH>` | A converter script that produced the files; its hash is kept as provenance |  |  |
| `--dry-run` | Show what would be imported without writing anything |  |  |
| `--max-bytes <SIZE>` | Refuse any file, or decompressed archive, larger than this (bytes, or with a K, M or G suffix) |  | `1G` |

### `evalsig items`

List the imported items

No options.

### `evalsig ledger`

Read, verify or anchor a ledger

No options.

### `evalsig ledger show`

List the entries of a ledger

| Option | Description | Values | Default |
|---|---|---|---|
| `--ledger <FILE>` | The ledger file (default: the project's agent-side ledger) |  |  |
| `--last <N>` | Show only the last N entries |  |  |

### `evalsig ledger verify`

Check the hash chain, and optionally the head against an anchor

| Option | Description | Values | Default |
|---|---|---|---|
| `--ledger <FILE>` |  |  |  |
| `--side <SIDE>` | Which ledger this is; entries of the other ledger or its kinds are reported | `agent`, `referee` |  |
| `--anchor <HASH>` | A head hash recorded elsewhere earlier (git, CI log, pull request). A truncated or rebuilt chain does not contain it |  |  |

### `evalsig ledger anchor`

Print the head hash to record outside the ledger

| Option | Description | Values | Default |
|---|---|---|---|
| `--ledger <FILE>` |  |  |  |

### `evalsig map`

Onboard records of an unknown shape: probe them and write a mapping

No options.

### `evalsig map probe`

Show the shape of a file: per path, types, counts and distinct values, with key and replicate candidates. Free text and secret-like values are never printed

| Option | Description | Values | Default |
|---|---|---|---|
| `<FILE>` (required) |  |  |  |
| `--no-values` | List no values at all, not even short categorical ones |  |  |
| `--max-bytes <SIZE>` | Refuse a file, or decompressed archive, larger than this |  | `1G` |

### `evalsig map draft`

Draft a mapping for a file. Choices that change results stay `DECIDE`, listed with their options and effects under `decisions_needed`

| Option | Description | Values | Default |
|---|---|---|---|
| `<FILE>` (required) |  |  |  |
| `--out <FILE>` (required) | Where to write the mapping |  |  |
| `--force` | Replace an existing file |  |  |
| `--max-bytes <SIZE>` |  |  | `1G` |

### `evalsig map test`

Show what a mapping loses on a file: coverage, unused paths, key integrity, value distribution, capabilities, and `<mapping>.tests.jsonl` expectations

| Option | Description | Values | Default |
|---|---|---|---|
| `<MAPPING>` (required) |  |  |  |
| `<FILES>` (required) |  |  |  |
| `--arg <NAME=VALUE>` | A value the mapping reads with `{ arg = "NAME" }`; repeat for several |  |  |
| `--max-bytes <SIZE>` |  |  | `1G` |

### `evalsig map decide`

Record a person's choice for one decision of a mapping, in the file and as a `mapping_decision` ledger line

| Option | Description | Values | Default |
|---|---|---|---|
| `<ID>` (required) | The decision: item_key, value_semantics, value, output_type, value.none, replicate_kind, missing.reason or positive |  |  |
| `<CHOICE>` (required) | The chosen option, as `decisions_needed` describes it |  |  |
| `--by <WHO>` (required) | Who decided (for example user:alice); kept in the file and the ledger |  |  |
| `--mapping <FILE>` | The mapping file |  | `mapping.toml` |

### `evalsig migrate`

Rewrite snapshots in an earlier record format in the current one

| Option | Description | Values | Default |
|---|---|---|---|
| `--dry-run` | List the snapshots that would be migrated without writing anything |  |  |

### `evalsig power`

Sample sizes, the minimum detectable difference and a gate's pass probability

| Option | Description | Values | Default |
|---|---|---|---|
| `--zero-events-below <U>` | Items to claim a disagreement rate below this bound when none is observed |  |  |
| `--rate <P>` | Items to measure a rate near P to within --half-width |  |  |
| `--half-width <W>` |  |  |  |
| `--delta <DELTA>` | The accuracy difference to detect between two systems on the same items |  |  |
| `--discordance <PSI>` | The share of items only one of the two systems gets right (ψ) |  |  |
| `--power <POWER>` |  |  | `0.8` |
| `--alpha <ALPHA>` |  |  | `0.05` |
| `--one-sided` | Test one-sided, as gates do |  |  |
| `--budget <BUDGET>` | Submissions a gate allows: the level becomes alpha / budget |  | `1` |
| `--n <N>` | With --discordance: the difference these many items detect, and with --delta the probability of passing |  |  |
| `--gate <FILE>` | Take the level, sidedness and budget from these gate conditions (gate.toml or an approved copy): the primary condition's level and sidedness, and the holdout budget |  |  |
| `--cluster-size <M>` | Mean cluster size and intra-cluster correlation for the design effect |  |  |
| `--icc <RHO>` |  |  |  |

### `evalsig preregister`

Draft gate conditions: check them, name them by hash and record the draft

| Option | Description | Values | Default |
|---|---|---|---|
| `--draft <FILE>` | Draft these conditions (gate.toml) |  |  |
| `--yes` | Confirm the draft; agent mode requires it (D-018) |  |  |

### `evalsig preregister approve`

As the referee: resolve and pin a draft into approved conditions

| Option | Description | Values | Default |
|---|---|---|---|
| `<DRAFT>` (required) | The draft's hash, as `preregister --draft` reported it |  |  |
| `--holdout <HASH>` (required) | The holdout the gate will use, by its hash |  |  |
| `--from <DIR>` | The project whose .evalsig/drafts/ holds the draft (default: --project) |  |  |
| `--mapping <FILE>` | The mapping file or converter that makes the candidate's records; its hash is pinned |  |  |
| `--referee <DIR>` | The referee's home (default: EVALSIG_REFEREE) |  |  |

### `evalsig preregister show`

As the referee: print approved conditions as gate reads them

| Option | Description | Values | Default |
|---|---|---|---|
| `<APPROVED>` (required) | The approved conditions' hash |  |  |
| `--referee <DIR>` | The referee's home (default: EVALSIG_REFEREE) |  |  |

### `evalsig purge`

Remove a content body and the free-text fields of the items that name it

| Option | Description | Values | Default |
|---|---|---|---|
| `<BODY>` (required) | The body to remove, as its `blake3:` or `sha256:` reference |  |  |
| `--reason <REASON>` (required) | Why the body is removed; kept in the ledger |  |  |
| `--yes` | Confirm the removal, which cannot be undone |  |  |

### `evalsig referee`

Create a referee home with a new age identity (D-014)

No options.

### `evalsig referee init`

Create the referee's home: an age identity, the referee ledger and its state

| Option | Description | Values | Default |
|---|---|---|---|
| `<DIR>` (required) |  |  |  |

### `evalsig rebuild`

Recreate the SQLite working copy from the canonical snapshots

No options.

### `evalsig report`

The default-tier metrics of one run

| Option | Description | Values | Default |
|---|---|---|---|
| `<RUN>` (required) | The run to report |  |  |
| `--task <TASK>` | The judgment task |  | `default` |
| `--estimand <ESTIMAND>` | The estimand: single_shot, majority@k, any@k or all@k (default from the configuration) |  |  |
| `--threshold <THRESHOLD>` | Decision threshold for score outputs; for score runs, also the share at or above it |  |  |
| `--labels-as-of <TIME>` | Use the labels as they were at this RFC 3339 time |  |  |
| `--reference <REFERENCE>` | How several raters' labels become the reference (default from the configuration) | `single`, `majority`, `soft`, `latent`, `human` |  |
| `--by <STRATUM>` | Also report each value of this stratum. `reference_agreement` splits the items two or more human raters labelled into `unanimous` and `split` |  |  |
| `--items` | Include one row per item |  |  |
| `--k-curve` | Also report majority@k, any@k and all@k for k = 1 … the fewest replicates of any item, with the expected cost per item |  |  |
| `--features` | Add each feature's likelihood ratio and class |  |  |
| `--proxy <RATER>` | With --reference human: correct the human-scale value with this model rater's labels on the items humans did not label (PPI++) |  |  |
| `--allow-unexplained-missing` | Continue although some missing values have no explained reason |  |  |
| `--fail-on-secret` | Stop when content looks like a secret instead of warning |  |  |

### `evalsig runs`

List the imported runs

No options.

### `evalsig seal`

As the referee: encrypt a holdout's labels to the referee's recipient

| Option | Description | Values | Default |
|---|---|---|---|
| `--holdout <HASH>` (required) | The holdout to seal, by the hash `split --holdout` reported |  |  |
| `--agent-project <DIR>` (required) | The agent's project, which must hold no label of a holdout item |  |  |
| `--referee <DIR>` | The referee's home (default: EVALSIG_REFEREE) |  |  |

### `evalsig skill`

Install the evalsig Agent Skill, or check an installed one against this binary

No options.

### `evalsig skill install`

Write the evalsig Skill where the agent reads skills

| Option | Description | Values | Default |
|---|---|---|---|
| `--target <TARGET>` |  | `claude`, `codex`, `generic` | `claude` |
| `--scope <SCOPE>` |  | `project`, `user` | `project` |
| `--dir <DIR>` | Write to (or check) `<DIR>/evalsig` instead; required with --target generic |  |  |
| `--force` | Replace an installed copy that differs |  |  |

### `evalsig skill check`

Compare an installed Skill with the one this binary writes

| Option | Description | Values | Default |
|---|---|---|---|
| `--target <TARGET>` |  | `claude`, `codex`, `generic` | `claude` |
| `--scope <SCOPE>` |  | `project`, `user` | `project` |
| `--dir <DIR>` | Write to (or check) `<DIR>/evalsig` instead; required with --target generic |  |  |

### `evalsig split`

Split the items into dev and validation by cluster, and record the split

| Option | Description | Values | Default |
|---|---|---|---|
| `--method <METHOD>` | How to assign clusters (default: time when every item has occurred_at, else random). --assign gives the assignment instead | `time`, `random` |  |
| `--validation <SHARE>` | Share of the items, counted by whole clusters, that goes to validation |  | `0.3` |
| `--stratum <STRATUM>` | For random splits: keep this item stratum's shares on both sides |  |  |
| `--seed <SEED>` | Seed of a random split (default: the configured seed) |  |  |
| `--assign <FILE>` | A JSON object mapping each item ID to "dev" or "validation" |  |  |
| `--within <FILE>` | The referee's remaining.json: split only these items, leaving the sealed holdout out |  |  |
| `--holdout` | As the referee: cut the holdout from all items and write the remaining item IDs, without labels, for the agent |  |  |
| `--share <SHARE>` | With --holdout: the share of the items, by whole clusters, held out |  | `0.2` |
| `--out <FILE>` | With --holdout: where to write remaining.json for the agent |  |  |
| `--referee <DIR>` | The referee's home (default: EVALSIG_REFEREE) |  |  |

### `evalsig splits`

List the recorded dev / validation splits

No options.

### `evalsig schema`

Print a published JSON Schema (`records`, `ledger-entry`, `mapping`, `gate`, `envelope`)

| Option | Description | Values | Default |
|---|---|---|---|
| `<NAME>` (required) |  |  |  |
| `--raw` | Print only the schema document, without the envelope |  |  |
| `--emit <LANG>` | Print a standard-library-only records/1 writer instead (python, typescript) |  |  |

### `evalsig recipes`

The steps of common tasks, and how to get what evalsig needs from a source

No options.

### `evalsig recipes compare`

Compare a new version with the current one

No options.

### `evalsig recipes sample-size`

How many items a question needs

No options.

### `evalsig recipes gate`

Decide adoption on a sealed holdout with preregistered conditions

No options.

### `evalsig recipes improve-loop`

Improve a system while looking at evaluations, without fitting the holdout

No options.

### `evalsig recipes dspy`

Use evalsig to judge a DSPy program's optimisation

No options.

### `evalsig recipes acquire`

Steps to get judgments, replicates and labels out of a source

| Option | Description | Values | Default |
|---|---|---|---|
| `<SOURCE>` (required) |  | `inspect`, `promptfoo`, `langfuse`, `csv` |  |

### `evalsig validate`

Run validity checks only: on a records file, or on the imported data

| Option | Description | Values | Default |
|---|---|---|---|
| `--records <FILE>` | Check this `records/1` file instead of the project's data |  |  |
| `--list-checks` | List every check with its default action and override flag |  |  |
| `--allow-unexplained-missing` | Continue although some missing values have no explained reason |  |  |
| `--fail-on-secret` | Stop when content looks like a secret instead of warning |  |  |

### `evalsig threshold`

The cost-optimal decision threshold of a run and its sensitivity to the cost ratio

| Option | Description | Values | Default |
|---|---|---|---|
| `<RUN>` (required) | A run whose judgments are levels |  |  |
| `--task <TASK>` | The judgment task |  | `default` |
| `--estimand <ESTIMAND>` | The estimand: single_shot, majority@k, any@k or all@k (default from the configuration) |  |  |
| `--threshold <THRESHOLD>` | Decision threshold for score outputs; for score runs, also the share at or above it |  |  |
| `--labels-as-of <TIME>` | Use the labels as they were at this RFC 3339 time |  |  |
| `--reference <REFERENCE>` | How several raters' labels become the reference (default from the configuration) | `single`, `majority`, `soft`, `latent`, `human` |  |
| `--ratio <R>` | The cost ratio r = C_FN / C_FP for the threshold (default: the matrix's corners, C[positive][lowest level] / C[negative][highest level]) |  |  |
| `--against <RUN>` | Also the expected-cost difference with this run (at its decision rule) at both ends of the configured ratio range |  |  |

### `evalsig trend`

A run's metrics per period of occurred_at, compared with a baseline period

| Option | Description | Values | Default |
|---|---|---|---|
| `<RUN>` (required) | The run to follow over time |  |  |
| `--task <TASK>` | The judgment task |  | `default` |
| `--estimand <ESTIMAND>` | The estimand: single_shot, majority@k, any@k or all@k (default from the configuration) |  |  |
| `--threshold <THRESHOLD>` | Decision threshold for score outputs; for score runs, also the share at or above it |  |  |
| `--labels-as-of <TIME>` | Use the labels as they were at this RFC 3339 time |  |  |
| `--reference <REFERENCE>` | How several raters' labels become the reference (default from the configuration) | `single`, `majority`, `soft`, `latent`, `human` |  |
| `--by <BY>` | The period length | `day`, `week`, `month` | `week` |
| `--since <TIME>` | Only items that occurred at or after this RFC 3339 time |  |  |
| `--baseline-period <PERIOD>` | The period the others are compared with (default: the first) |  |  |

### `evalsig verify`

Recompute a saved result from its snapshots and configuration

| Option | Description | Values | Default |
|---|---|---|---|
| `<REPORT>` (required) | A saved result (a file under `.evalsig/reports/`, or one produced with --format json) |  |  |

### `evalsig version`

Show the tool, record-format and output-schema versions

No options.

## Verdict codes

| Code | Meaning |
|---|---|
| `pass` | every approved condition holds; say the conditions were met |
| `fail` | a condition does not hold; say which condition was not met |
| `improved` | the interval of the difference excludes 0 on the better side and the point difference reaches the minimum effect; say it improved, with the interval |
| `negligible` | the interval lies within the minimum effect, or the difference is real but small; say there is no practical difference, neither improved nor worsened |
| `worsened` | the interval of the difference excludes 0 on the worse side; say it worsened, with the interval |
| `indistinguishable` | none of the above: the interval crosses 0; say the difference cannot be told apart, neither improved nor worsened |
| `drifted` | over time, a period's interval left the baseline period's; say which period and the interval |
| `stable` | over time, no period left the baseline period |
| `human_parity` | the system agrees with people as well as people do (alternative annotator test); quote the winning rate |
| `below_human` | the system does not agree with people as well as people do |
| `insufficient_data` | too few items to decide; quote the required count from the power command in next |
| `blocked` | a validity check or a referee check stopped the command; quote the reason and how to clear it (the gate has no override) |
| `needs_decisions` | a mapping draft has decisions a person must make; show the choices and their effects and ask, without choosing |
| `budget_exhausted` | the holdout's budget is spent; say so and ask a person to decide on the budget |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success, or the gate or check passed |
| 1 | the gate or check failed |
| 2 | the holdout budget is exhausted |
| 3 | conditions are invalid or do not match the approval, or the candidate failed the referee's checks |
| 4 | too few items to decide |
| 5 | stopped by a validity check |
| 64 | usage error |
