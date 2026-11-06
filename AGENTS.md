# evalsig — contributor and agent rules

evalsig analyses the evaluation records of non-deterministic judges (LLM
judges, classifiers, rule sets) with statistics and signal detection theory.
It ships as a Rust CLI and a local web dashboard. These rules apply to human
contributors and to AI coding agents alike.

## Repository map

| Path | Contents |
|---|---|
| `crates/evalsig-core` | Data types, metrics and validity checks. Pure computation, no I/O |
| `crates/evalsig-store` | Objects, ledgers, canonicalization and hashing, sealing |
| `crates/evalsig-adapters` | Ingestion adapters (Inspect, promptfoo, declarative mapping) |
| `crates/evalsig-project` | The `.evalsig/` project: data in force, analyses, provenance, import, published JSON Schemas |
| `crates/evalsig-referee` | The referee: its home, the holdout cut and its exclusions, sealing, gate conditions and approval, the gate and the Ladder, `gate serve` |
| `crates/evalsig-dashboard` | Local web dashboard: screens, server, HTML and SVG rendering |
| `crates/evalsig-cli` | The `evalsig` binary: argument parsing, rendering, agent output, generated help and Skill |
| `docs/decisions.md` | Decision ledger (`D-NNN`). Read its header before adding an entry |
| `docs/glossary.md` | Glossary. CLI output, reports and docs use only these terms |
| `tests/fixtures/` | Reference values and canonicalization vectors (generated) |
| `scripts/` | Fixture generators and cross-language verifiers |
| `fuzz/` | cargo-fuzz targets for untrusted input (nightly, its own workspace) |

Task IDs (`T-NNN`) in commit trailers refer to the maintainers' planning
backlog, which is kept outside the repository.

## Working in this repository

- Read the relevant implementation, callers and tests before editing. Keep
  changes within the request; avoid unrelated refactoring or dependencies.
- Preserve other contributors' uncommitted and staged work. Stage only the
  files or hunks you own.
- Make ordinary implementation decisions autonomously. Choices about
  architecture, dependencies, the data model, public APIs or authentication go
  through `docs/decisions.md` first (see Commit messages).
- For review or investigation requests, report findings without modifying
  code unless fixes are requested.
- Put local reports, screenshots and scratch artifacts in `.output/` or an OS
  temporary directory, and keep them out of commits.
- Write repository documentation, code comments and commit messages in
  English. Respond to the user in their preferred language.

### Commit messages

Use this format for every commit. Don't make summary-only commits such as
`update`, `fix` or `wip`.

```text
<type>(<scope>): <one-line summary>

Why: <background in one to three sentences>
How: <the approach taken; the core of the decision, not implementation details>
Alternatives: <options considered and not taken, and why; omit if none>
Impact: <debt taken on or constraints this creates; omit if none>

Task: T-NNN
Decision: D-NNN
Assisted-by: <agent>:<model ID>
```

- Use Conventional Commits types such as `feat`, `fix`, `docs`, `refactor`,
  `test`, `chore` and `ci`, with an optional package scope such as `core`,
  `store`, `adapters`, `cli` or `dashboard`.
- `Task:`, `Decision:` and `Supersedes:` are git trailers with one reference
  per line; several are allowed.
- A change that involves a decision always carries `Decision:`. A change with
  a related task always carries `Task:`.
- To settle a decision before any code depends on it, make an empty commit
  (`git commit --allow-empty`) whose body gives the reasons and carries the
  `Decision:` trailer.
- An agent-assisted commit ends with exactly one `Assisted-by:` trailer, for
  example `Assisted-by: Claude:claude-opus-5-5`. Add no `Co-Authored-By:` or
  `Signed-off-by:` trailers and no generated-by notice.
- A squash merge keeps the body and the trailers in the squashed message.

Look up history with:

```sh
git log --grep='Decision: D-003'      # everything about one decision
git log --grep='Task: T-012' --stat   # what one task changed
```

### Splitting a ledger

`docs/decisions.md` and `docs/glossary.md` are single files. When either meets
the split signal in its header, report it instead of starting. After approval:

1. Do it in one go. A staged migration scatters information across two places.
2. Never renumber IDs, since commit trailers would stop resolving.
3. Create `INDEX.md` (one entry per line) in the new directory and move the
   ledger's rules there.
4. Rewrite this section and the repository map above for the new layout.
5. Keep the migration commit free of anything other than the move.

## OSS and information handling

evalsig is published as public OSS. Treat source, examples, commit messages,
issues and release artifacts as public.

- Use fictional or sanitized examples. Never add employer or customer
  material, internal domains, private project names, credentials or tokens.
- Evaluation records, eval cases and baselines built from private data stay in
  the owner's private repositories, never in this one.
- Avoid personal email addresses, home-directory paths and machine details in
  fixtures and documentation. Use repository-relative paths, temporary
  directories and reserved example domains such as `example.com`.
- Only contribute code and assets whose redistribution rights are established,
  and check the license of a runtime dependency before adding it.
- Inspect staged changes before describing them as safe to publish. Don't rely
  on a developer's global ignore configuration.
