# The judge: threat model and deployment

evalsig can act as a judge (referee) for an agent that improves a system by
itself. The agent may run any command in its own environment; the judge runs
with other privileges, holds the age secret key, the sealed holdout labels and
its own ledger, and decides pass or fail on conditions a person approved. This
page states what that arrangement protects, what it cannot protect, and how to
deploy it. The tests named below live in `crates/evalsig-cli/tests/judge_paths.rs`
(T-127) unless another file is given.

## Assumptions

- The agent controls its working environment: `.evalsig/`, the configuration,
  the source files and their git history. It writes the candidate's records
  itself.
- The judge runs as another OS user, in CI or on another machine. The agent
  never has the secret key, the sealed labels or write access to the judge's
  ledger and state.
- The judge trusts only what it fixed itself: the approved conditions with
  every setting resolved (`approved/<hash>.toml`), its own ledger and the sealed
  labels. Everything else — candidate records, the agent-side ledger, the
  project configuration, source files — is a claim it checks.

## What the judge protects

| Protection | How | Task | Test |
|---|---|---|---|
| Holdout labels never enter the agent's environment | The judge cuts the holdout and seals its labels; the agent receives item IDs only; `seal` refuses when agent-side snapshots already hold holdout labels | T-129, T-084 | `remaining_export_has_no_labels`, `seal_refuses_agent_side_holdout_labels` |
| Passing by leaving items out of the candidate's records | Coverage of the holdout item set is checked before labels are read; absent items are filled with the system's worst judgment | T-125 | `judge_item_omission` |
| Passing by marking judgments missing | Every missing judgment counts as the worst, whatever its reason | T-125 | `judge_missing_reason_manipulation` |
| Passing by rewriting item attributes (weight, cluster, strata, inclusion probability, input) | Approval pins the holdout items' attributes by hash | T-086 | `judge_attribute_rewrite` |
| Passing by rewriting the configuration (positive definition, missing handling, level, seed) | Approval resolves every setting; `gate` reads nothing from `config.toml` | T-086 | `judge_config_rewrite`, `gate_reads_no_project_config` |
| Passing by rewriting the mapping (`value_map` and the rest) | Approval pins the hash of the mapping or converter; a candidate made with another fails with exit 3 | T-086, T-125 | `judge_value_map_rewrite` |
| Passing with another grader | The candidate's `run.grader` must match the approval; exit 3 otherwise | T-125 | `judge_grader_version_mismatch` |
| Resetting the budget by rewriting and re-approving the conditions | The budget belongs to the sealed holdout, not to the conditions | T-125 | `judge_reapproval_keeps_budget` |
| Spending nothing by resubmitting a candidate | The same snapshot hash returns the earlier result without using budget | T-125 | `judge_same_candidate_resubmission` |
| Inferring labels from "too few items" or from warnings | `min_items` counts label-independent paired items; label-dependent warnings are recorded in the judge ledger, not returned | T-125 | `judge_min_items_boundary`, `judge_warnings_withheld` |
| Relaying a forged pass | The judge writes results to paths it owns and to its ledger; people read only those | T-089 | `judge_forged_pass_is_not_in_judge_ledger` |

The fine-grained information the judge returns is limited further by the
Ladder (metrics spec §11.3, T-126), whose defaults await D-015.

## What the judge cannot protect

- **Changing the grader.** The version check stops mix-ups, not a grader the
  agent has altered; only a judge that runs the grader itself could.
- **Hiding holdout results of `score` runs.** When the grader's output is the
  result, the agent sees its own scores in its own run. Judge-side grading is
  planned for v2.
- **Model-derived reference labels.** They can be regenerated from the inputs,
  so sealing them protects nothing; `gate` stops on such items
  (`model_only_holdout_reference`).
- **Selecting replicates.** An agent that keeps only its best replicates out of
  many cannot be detected.
- **Fitting the validation split.** Its labels are in the agent's environment;
  `validation_uses` and its warning help a cooperative agent, they are not a
  guard. Guarding it would need a sealed validation split and a judge-side
  `compare` with a budget (v2).
- **Tampering with the agent-side ledger.** It is evidence, not proof; the
  judge copies its head into each `gate` line, and only the judge ledger counts.

## Removing holdout labels from the agent's environment

Cut and seal the holdout on the judge side before the agent sees the data:

1. Import the labelled data into the judge's project, run `split --holdout`
   and `seal` there, and hand the agent `remaining.json` (item IDs only).
2. Give the agent only the records of the remaining items, or records without
   any `label` of holdout items. `seal` refuses if agent-side snapshots hold
   holdout labels.
3. Keep the original label files out of the agent's checkout. If they were
   ever committed, rewrite that history in a copy the agent never sees
   (for example with `git filter-repo --path <labels> --invert-paths`), and
   rotate the repository the agent uses; the old history still holds the
   labels.

## Deployments

| Form | Judge | Agent submits with | Conditions |
|---|---|---|---|
| Local | `evalsig gate serve` under a separate OS user that owns the key, the judge ledger and `approved/` | `evalsig gate submit` over a Unix socket, or token HTTP on 127.0.0.1 | The agent's user cannot read the judge user's files |
| CI | A workflow job with the key in an environment-protected secret runs `evalsig gate` and posts the result | A pull request carrying the candidate's records | The workflow and `approved/` come from the base branch (`pull_request_target` with a base-ref checkout, or a required check in another repository); the judge ledger and state live where pull requests cannot write (a protected branch or an outside store); fork pull requests get no secret. See `examples/ci/` (T-090) |
| Remote | `gate serve` on another machine, behind HTTPS | `gate submit` with a token | As local, plus the transport's own authentication |

Running `gate` with the agent's own privileges, key included, makes the seal
meaningless; `gate` then warns that the seal is void and records it in the
ledger.
