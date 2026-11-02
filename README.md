# evalsig

evalsig reads the evaluation records of judges that do not always give the
same answer — LLM judges, classifiers, rule sets — and answers, with
intervals:

- how well the judge separates what humans call positive from what they call
  negative, and where it starts saying "positive";
- how often its answers split when the same input is judged again;
- whether a change really helped, or only moved the decision line or fell
  within noise.

It is an analysis-only command-line tool with a local web dashboard. It never
calls an AI model. It treats human reference labels as noisy too: it measures
how much they vary, states what each estimate targets, and compares systems
and humans under the same rule.

When an AI agent improves a system in a loop, evalsig can act as a referee
that keeps the holdout labels away from the agent: it seals them, counts every
use, and decides pass or fail under conditions a human approved in advance.

## Status

Pre-release. The command-line interface, the record format and the output
envelope are under construction and may change until 1.0.

## What it answers

| Question | Command |
|---|---|
| Can this ship? | `gate` (preregistered conditions on a sealed holdout), `check` (absolute thresholds with intervals) |
| Did the change help? | `compare` (paired intervals, minimum effect, transition table, pairwise win rate) |
| Better at telling cases apart, or just a moved threshold? | `report` (ROC and AUC, d′ and criterion c) |
| How good is it once human disagreement is included? Is it as close to humans as humans are to each other? | `agree` (reference modes, rater design, alternative annotator test) |
| How many items are enough? | `power` |
| Is production drifting? | `trend`, `compare --unpaired` |

## Non-goals

- Calling AI models or running evaluations. Use Inspect, promptfoo or your own
  harness, then import the records.
- Prompt management, trace observation or a labelling interface (v1).
- A multi-user server or a cloud service.

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
