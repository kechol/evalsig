# Contributing to evalsig

Thank you for helping. Read [AGENTS.md](AGENTS.md) first: it holds the working
rules, the commit message format and the information-handling rules that apply
to every change, whether written by a person or with an AI agent.

## Before you start

- For anything larger than a small fix, open an issue first so the approach
  can be agreed.
- Changes to architecture, dependencies, the record format, public APIs, the
  CLI output envelope or authentication need an entry in
  [docs/decisions.md](docs/decisions.md) before code depends on them.
- Use the terms in [docs/glossary.md](docs/glossary.md) in user-facing output
  and documentation.

## Development

The toolchain is pinned in `rust-toolchain.toml`; `rustup` installs it on
first use.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

All three must pass before a pull request is reviewed. Numerical code is
tested against the reference values in `tests/fixtures/`; see
[scripts/README.md](scripts/README.md) for how they are generated.

## Pull requests

- Keep one purpose per pull request and per commit.
- Describe why the change is needed, what it changes, and how you verified it.
- Use synthetic or sanitized data in tests and examples.

## Licence

By contributing, you agree that your contribution is dual licensed under the
MIT and Apache-2.0 licences, as described in [README.md](README.md#licence).
