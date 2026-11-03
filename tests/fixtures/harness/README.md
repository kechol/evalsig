# Harness fixtures

Real outputs of Inspect and promptfoo, used as inputs of the Inspect and promptfoo
adapter tests. They come from tiny evaluations whose models call no AI, so they hold no
model output and no personal data.

| File | Produced by | Contents |
|---|---|---|
| `inspect/probe.eval` | `inspect eval inspect/probe_task.py --model mockllm/model` (inspect-ai 0.3.286) | The zip log: `header.json`, `samples/<id>_epoch_<n>.json`, `summaries.json`, `reductions.json`, `_journal/` |
| `inspect/probe.json` | The same with `--log-format json` | The JSON log of a second run of the same task |
| `inspect/probe_task.py` | — | The task: samples `s1` and `s2` (metadata `stratum` a and b), `includes()` scorer, `epochs=2` |
| `promptfoo/out.json` | `promptfoo eval -o out.json --repeat 2` (promptfoo 0.124.1, `echo` provider) | `results.version` 3; two tests × two repeats; the second test fails its assertion on purpose |
| `promptfoo/promptfooconfig.yaml` | — | The promptfoo configuration |

Expected counts:

- Inspect: 2 samples × 2 epochs = 4 sample records; `includes` scores `s1` as `I` and
  `s2` as `C` in both epochs (the mock model answers "Default output from mockllm/model").
- promptfoo: 4 results (`testIdx` 0–3), tests with `vars.q` "yes" (passes) and "no"
  (fails), each twice; promptfoo does not record which repeat a result is.

## Provenance and checks

- Generated on 2026-11-03 with `scripts/gen_harness_fixtures.sh` in a temporary directory.
  Inspect ran with `mockllm/model`, promptfoo with the `echo` provider, and promptfoo's
  telemetry and update checks were off.
- promptfoo records the working directory and the machine. The generator replaces
  `config.basePath` with `.`, `config.outputPath` with `["out.json"]` and
  `metadata.nodeVersion`, `metadata.platform` and `metadata.arch` with `redacted`; nothing
  else is edited. The Inspect logs name only `probe_task.py` and the package versions.
- The key paths and types of both files match the structure observed during the format
  research (`scripts/json_shape.py` on the Inspect JSON dump and on `out.json`).

## Regenerating

```sh
scripts/gen_harness_fixtures.sh            # both harnesses
scripts/gen_harness_fixtures.sh inspect    # one of them
```

The script needs uv, npm and jq, downloads the pinned inspect-ai and promptfoo versions,
and overwrites the files above. IDs and timestamps change on every run, so adapter tests
assert on counts and fields, not on bytes. Update the versions in the script and this
README together.

## Licences

Inspect (inspect-ai, UK AI Security Institute) and promptfoo are MIT-licensed. These
files are outputs of running them on inputs written for this repository, not copies of
their source.
