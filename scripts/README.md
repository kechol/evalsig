# Scripts

Fixture generators and cross-language verifiers. They run with the pinned Python
environment in this directory:

```sh
uv run --project scripts scripts/<script>.py
```

| Script | Purpose |
|---|---|
| `evalsig_canon.py` | Canonical JSON bytes (RFC 8785 + NFC) and `sha256:` / `blake3:` references. Standard library only, except BLAKE3 |
| `gen_canonicalization_vectors.py` | Writes `tests/fixtures/canonicalization-vectors.json` |
| `reference_values.py` | Writes `tests/fixtures/reference-values.json` from scipy, closed forms and published values |
| `sample_size.py` | Writes `docs/sample-size.md`, the sample-size tables that `evalsig_core::power` also renders |
| `check_fixtures.py` | Regenerates both fixtures and `docs/sample-size.md` in a temporary directory and compares them with the committed files, and checks `examples/custom-harness/records.out.jsonl` against `convert_runlog.py` |
| `gen_harness_fixtures.sh` | Runs Inspect and promptfoo with models that call no AI and writes `tests/fixtures/harness/` (see its README) |
| `json_shape.py` | Prints the key paths, types and example values of a JSON or JSONL file as a Markdown table |
| `verify_store.py` | Checks a `.evalsig/` directory or ledger file without evalsig: ledger chains, object names and canonical form, saved report hashes |
| `cross_verify.sh` | Builds stores with the Rust binary, checks them with `verify_store.py` and confirms it rejects tampered copies (CI job `cross-verify`) |
| `smoke_test.sh` | End-to-end smoke test of the binary over the examples |
| `gate_two_users.sh` | Runs `gate serve` and `gate submit` between the OS users `referee` and `agent` on a disposable Linux machine (CI job `gate-two-users`) |
| `gate_serve_docker.sh` | Builds evalsig in a Rust container and runs `gate_two_users.sh` there |
| `check_changelog.sh` | Refuses a release tag whose version has no dated section in `CHANGELOG.md` (run by the release workflow) |
| `sim_latent_reference.py` | Simulates the bias and interval coverage of the `soft` and `latent` references (D-016) |
| `sim_ladder.py` | Simulates the false pass rate under an adaptive agent and the share of real improvements that pass, by holdout size, budget and Ladder step (D-015) |
| `sim_rare_rate_bounds.py` | Compares bootstrap and Clopper-Pearson upper bounds of rare rates (metrics §1) |
| `sim_percentile_tail.py` | Measures the null rejection rate of the percentile bootstrap, exact McNemar and Wald bounds of a paired difference, and the bootstrap quantile's Monte Carlo spread (metrics §7, §8) |
| `sim_lord.py` | Prints LORD++'s per-test levels and the smallest detectable difference at those levels (metrics §7.2) |
| `sim_nested_bootstrap.py` | Compares item-only and nested bootstrap standard errors and coverage with replicates (metrics §8) |
| `sim_agreement_rule.py` | Compares system–majority, system–rater and rater–rater κ at equal accuracy (metrics §9.1) |
| `sim_rater_resampling.py` | Compares item and rater resampling with the standard error over new raters in a crossed design (metrics §9.3) |

## Regenerating the fixtures

The fixtures are generated files; edit the generators, never the JSON.

```sh
uv run --project scripts scripts/gen_canonicalization_vectors.py
uv run --project scripts scripts/reference_values.py
uv run --project scripts scripts/sample_size.py
uv run --project scripts scripts/check_fixtures.py
```

`check_fixtures.py` requires the canonicalization vectors to match byte for byte and
the reference values to match within 1e-12. The Rust tests compare their results with
the reference values within the tolerance stated in the file (1e-9). Dependency
versions are pinned in `pyproject.toml` and `uv.lock`; changing them is a fixture
change and needs `check_fixtures.py` to pass.
