# Quick start

This walk-through reaches a first report in under ten minutes. It uses the
synthetic example in the repository: 300 support tickets triaged by two
versions of a prompt (`triage-v1`, `triage-v2`), with reference labels from
two human raters. A Japanese version of this page is in
[quickstart.ja.md](quickstart.ja.md); both describe the same steps.

## 1. Install

With Rust 1.90 or later:

```sh
cargo install --locked evalsig-cli
```

Prebuilt binaries and the Homebrew formula are described in the
[README](../README.md#install).

## 2. Get the example

```sh
git clone https://github.com/kechol/evalsig
mkdir triage
cd triage
cp ../evalsig/examples/synthetic/records.jsonl .
```

`records.jsonl` holds the records evalsig reads (`records/1`): two runs, the
items, the reference labels and one judgment per item and run.

## 3. Create a project and import

```sh
evalsig init
evalsig import records.jsonl
```

`init` creates `.evalsig/` with a default `config.toml`. `import` stores the
records as a snapshot, named by its content hash, and writes a line to the
ledger. Every later result names the snapshot it was computed from.

## 4. Read the first report

```sh
evalsig report triage-v1
```

What to read first:

- `metrics`: accuracy, hit rate, false alarm rate and precision, each with its
  `value`, its 95% interval `ci` and the interval method. The items belong to
  threads (`cluster`), so the interval resamples threads (`bootstrap`).
- `n_missing` and `missing_bounds`: six judgments timed out. The bounds show the
  metric with those judgments counted as the worst and as the best value.
- `reference`: two raters labelled some items, so the reference is `soft` (each
  item counts by the share of raters who called it positive), and
  `rater_spread` shows the metric against each rater alone.
- `sdt`: the sensitivity index d′ and the criterion c, which separate "tells the
  classes apart" from "where it starts saying positive".

## 5. Compare the two versions

```sh
evalsig compare triage-v1 triage-v2
```

The first line is the verdict, for example `improved: accuracy: difference
+0.0533 (better is positive; 95% interval +0.0133 to +0.0933)`. The verdict is
`improved` only when the interval of the paired difference excludes 0 and the
difference is at least `min_effect`; otherwise it is `negligible`,
`indistinguishable`, `worsened` or `insufficient_data`. `transition` counts
the items that moved between correct and wrong, and the exit code follows the
verdict (see the [CLI reference](cli.md#exit-codes)).

## 6. Look at it in the browser

```sh
evalsig dashboard
```

The dashboard listens on 127.0.0.1 only and prints a URL with a token made
for this launch. It reads the project and never writes to it. Stop it with
Ctrl-C. `evalsig report triage-v1 --format html > report.html` writes the same
report as one self-contained page.

## Next steps

- Import your own evaluations: `evalsig import --adapter inspect <log.eval>`
  for Inspect logs, `--adapter promptfoo` for promptfoo output or its local
  database, and `evalsig map probe`, `map draft` and `map test` to write a
  `mapping.toml` for anything else. `evalsig recipes acquire` lists the routes.
- Plan how many items you need: `evalsig power` and
  [sample-size.md](sample-size.md).
- Gate a release on a sealed holdout: [referee.md](referee.md).
- Let a coding agent drive evalsig: `evalsig skill install` and
  [agent-interface.md](agent-interface.md).
- Definitions of every metric and interval: [metrics.md](metrics.md); the full
  specification: [spec.md](spec.md).
