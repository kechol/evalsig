# Fuzz targets

cargo-fuzz targets for the readers of untrusted input (SPEC §13.1). They need a nightly
toolchain and live in their own workspace, outside the main build.

| Target | Input | What it reaches |
|---|---|---|
| `zip` | A zip archive | `limits::zip_entries` and the Inspect `.eval` reader |
| `zstd` | A Zstandard frame | `limits::zstd` |
| `json` | JSON or JSONL | The depth check, the Inspect and promptfoo readers, and `examples/mapping.toml` applied to the rows |

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd fuzz
cargo +nightly fuzz run json corpus/json seeds/json -- -max_total_time=60
```

`seeds/` holds the starting inputs (the harness fixtures and the custom-harness log).
`corpus/` and `artifacts/` are local; a crash input lands in `artifacts/<target>/`. The
scheduled `Fuzz` workflow runs each target for five minutes every week and keeps crash
inputs as workflow artifacts.
