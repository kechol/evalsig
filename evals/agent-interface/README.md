# Evaluation of the agent interface

Do agents quote evalsig's verdicts correctly, and how often do they overclaim?
This evaluation answers that with evalsig itself (agent-interface spec §6). It runs
LLM agents, so it runs locally, not in CI. Run it again whenever the Skill's text
changes, and record the result in `docs/agent-eval.md`.

## Contents

| Path | What it is |
|---|---|
| `scenarios.jsonl` | 20 scenarios: an evalsig output, a request, and what the report must and must not say |
| `fixtures/` | The evalsig outputs, valid against `evalsig schema envelope` |
| `score.py` | Scores reports and writes records/1 (standard library only) |
| `dummy/` | Reports of two scripted agents, `careful` and `overclaiming`, and their scored records; they test the pipeline, not an agent |

The kinds of scenario: comparisons whose difference cannot be told apart, negligible,
improved and worsened differences, too few items, mixed grader versions, a spent
budget, instructions hidden in evaluated data, and records that need a person's
decisions.

## Running it

1. Install the Skill where the agent reads it: `evalsig skill install`.
2. For each scenario, start a fresh session, give the agent the fixture's file and the
   request, and save its final report as `runs/<agent>/<scenario id>.txt`.
3. Score and analyse:

   ```sh
   python3 evals/agent-interface/score.py runs --out runs/records.jsonl
   evalsig --project runs init
   evalsig --project runs import runs/records.jsonl
   evalsig --project runs report <agent> --task overclaim
   evalsig --project runs report <agent> --task correct --by kind
   ```

   Each run has score semantics, so the rate is `pass_rate` with its interval: the
   overclaim rate on task `overclaim`, the correct-quote rate on task `correct`.
4. Read every report flagged as an overclaim or as not correct: matching is by
   phrase, so a careful report can be flagged wrongly. Record the overclaim rate
   with the upper end of its interval and the correct-quote rate in
   `docs/agent-eval.md`.

`dummy/` is regenerated and compared by `scripts/check_fixtures.py`.
