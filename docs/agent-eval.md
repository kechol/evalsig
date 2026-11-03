# Agent interface evaluation results

The evaluation in `evals/agent-interface/` measures whether coding agents using
evalsig's Skill quote verdicts correctly and how often they overclaim, for
example calling `indistinguishable` an improvement (agent-interface spec §6). It
runs LLM agents locally; see its README for the steps.

Each row records one run of all 20 scenarios. The overclaim rate is `pass_rate`
of task `overclaim` with the upper end of its 95% interval; the correct rate is
`pass_rate` of task `correct`. A Skill change is accepted only when the
overclaim rate's upper end does not rise above the last row's.

| Date | Agent and model | evalsig and Skill version | Overclaim rate (upper end) | Correct rate | Notes |
|---|---|---|---|---|---|

No run has been recorded.
