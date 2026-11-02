# Decisions

This ledger records choices about architecture, dependencies, the data model,
public APIs and authentication. Code that depends on a decision lands only
after the entry is `Accepted`.

## Rules for this file

- IDs (`D-NNN`) are permanent. Never renumber; append the next number after
  the highest in use.
- Status is one of `Proposed`, `Accepted` or `Superseded by D-NNN`. To change
  an accepted decision, add a new entry with a `Supersedes: D-NNN` line and
  set the old entry's status; do not rewrite the old entry's reasoning.
- Each entry has: Status, Date, Context, Decision, Alternatives and
  Consequences. Keep it to what a future reader needs to understand why.
- The commit that settles an entry carries `Decision: D-NNN`. If no code
  depends on it yet, the commit may be empty (`git commit --allow-empty`).
- **Split signal**: when this file exceeds 1,500 lines or 60 entries, report
  it and follow "Splitting a ledger" in `AGENTS.md`.

## D-001 Workspace layout, toolchain and baseline dependencies

- Status: Accepted
- Date: 2026-11-03
- Context: evalsig needs a pure, separately publishable statistics library and
  several I/O-heavy front ends. Most tasks need the same few general-purpose
  crates, and asking for a decision each time would add no information.
- Decision:
  - A Cargo workspace under `crates/` with five members:
    `evalsig-core` (types, metrics, validity checks; no I/O),
    `evalsig-store` (objects, ledgers, SQLite, canonicalization, hashing,
    sealing), `evalsig-adapters` (ingestion), `evalsig-cli` (binary
    `evalsig`) and `evalsig-dashboard` (web server, SVG).
  - Dependencies point inwards: `core` depends on no other member; `store`
    and `adapters` may depend on `core`; `cli` and `dashboard` may depend on
    everything below them.
  - Rust edition 2024. The development toolchain is pinned in
    `rust-toolchain.toml`; the MSRV is declared as `rust-version` in the
    workspace manifest and built in CI.
  - Baseline dependencies that any member may use without a further decision:
    `serde` (with `derive`), `serde_json`, `toml`, `thiserror` for library
    errors, and in the binary crates `clap` (with `derive`) and `anyhow`.
    Test-only crates (`proptest`, `assert_cmd`, `predicates`, `tempfile`) need
    no decision either. Every other runtime dependency goes through this file.
- Alternatives: a single crate (rejected: `evalsig-core` must be publishable
  without the CLI, SQLite and web stacks); more, finer crates (rejected: no
  consumer needs the extra boundaries yet).
- Consequences: library crates return typed `thiserror` errors; only the
  binaries use `anyhow`. Raising the MSRV is a release-note item.

## D-002 Licence and documentation language

- Status: Accepted
- Date: 2026-11-03
- Context: evalsig is a personal open-source project meant to be embedded in
  other Rust tools and read by an international audience.
- Decision:
  - Every crate is licensed `MIT OR Apache-2.0`, with `LICENSE-MIT` and
    `LICENSE-APACHE` at the repository root. Contributions are accepted under
    the same terms.
  - English is the canonical language of all documentation, code comments,
    CLI output and commit messages. In v1, only `README` and the quick start
    also exist in Japanese (`README.ja.md`, `docs/quickstart.ja.md`); the
    English version wins where they differ.
- Alternatives: MIT only or Apache-2.0 only (rejected: the dual licence is the
  Rust ecosystem norm and lets users pick); translating all documents
  (rejected: two canonical copies drift).
- Consequences: dependencies must have licences compatible with
  `MIT OR Apache-2.0`; `cargo deny` enforces the allow list.
