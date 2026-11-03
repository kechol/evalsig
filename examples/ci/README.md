# The referee in GitHub Actions

`github-actions.yml` evaluates a pull request with promptfoo and lets evalsig
decide on the sealed holdout. It is an example to adapt: the repository names,
secrets and paths are placeholders. It passes `actionlint`.

## Preconditions

The seal protects nothing unless each of these holds (`docs/referee.md`).

- **The workflow comes from the base branch.** It runs on `pull_request_target`,
  so a pull request cannot change it. Branch protection on `main` requires review
  of changes to `.github/workflows/` and `.evalsig-gate/`.
- **Pull request code never runs next to a secret.** Only the `candidate` job runs
  the pull request's promptfoo configuration; it has no secrets, a read-only
  token and `persist-credentials: false`. The `gate` job takes the candidate's
  records as data and runs only evalsig.
- **The approved conditions come from the base branch.** `.evalsig-gate/approved`
  on `main` holds the hash of the approved conditions; the `gate` job checks out
  the base commit to read it, so a pull request cannot point the gate elsewhere.
  The approved file itself (`approved/<hash>.toml`) lives in the referee home.
- **The referee's state lives where pull requests cannot write.** The referee's
  project (its labelled records and the baseline) and its home (the ledger,
  `holdouts/`, `sealed/`, `approved/`, `results/`, `state/`) sit in a separate
  private repository, `example-org/evalsig-referee-state` here, reached with a
  deploy key that has write access to that repository only. The identity is not
  stored there.
- **The secrets sit in a protected environment.** `EVALSIG_REFEREE_IDENTITY` (the
  contents of the referee's `identity.txt`) and `REFEREE_STATE_DEPLOY_KEY` are
  secrets of the environment `referee`, which requires a reviewer and admits
  only `main`. On `pull_request_target` a fork's pull request runs with the base
  repository's secrets, so that reviewer is the protection: approve a gate run
  only after reading the pull request.
- **One gate run at a time.** The `concurrency` group runs one gate job at a time,
  so two submissions cannot both read the same budget. GitHub keeps one pending
  run per group and cancels an older pending one, which then has to be re-run.

## Setting up

1. On a trusted machine, as the referee: `evalsig referee init home`, import the
   labelled records into a project, `split --holdout` and `seal` (`docs/referee.md`).
2. Approve the agent's draft with `preregister approve`, and commit the hash it
   prints to `.evalsig-gate/approved` on `main`.
3. Commit the project and the home, without `identity.txt`, to the state
   repository. Store `identity.txt` as `EVALSIG_REFEREE_IDENTITY` in the
   environment `referee`, and keep no other copy where the agent can read it.

The gate step's exit code is the job's result: 0 pass, 1 fail, 2 budget spent, 3
refused, 4 too few items, 5 stopped by a validity check.
