# AGENTS.md

## Scope

- This file contains `nils-cli`-specific agent instructions only.
- Global agent behavior, response style, dispatch flow, clarification policy,
  commit policy, and shared tool rules are inherited from the fallback
  `AGENTS.md`.
- Do not duplicate global guidance here; add only local overrides or local
  document pointers.

## Local Source Of Truth

Open only the references relevant to the requested change:

- Maintenance principles and routine workflow: `DEVELOPMENT.md`
- Repository-only current-contract rule and expiring audit: `DEVELOPMENT.md#current-contracts-in-this-repository`
- Detailed setup, validation, coverage, and publishing reference:
  `docs/runbooks/workspace-maintenance-reference.md`
- Runtime dependency and degradation reference: `BINARY_DEPENDENCIES.md`
- CLI completion policy: `docs/runbooks/cli-completion-development-standard.md`
- New CLI crate standard: `docs/runbooks/new-cli-crate-development-standard.md`
- Crate docs placement policy: `docs/specs/crate-docs-placement-policy.md`
- Development log (append-only history): `docs/devlog/README.md`

## Development log writers

- Source `scripts/dev-env.sh` in every development/automation shell; this
  repository requires `DEVLOG_LAYOUT=fragments` and `devlog >=1.31.16`.
- Exceptions are a `semantic-commit` authoring call and, when the owner guard
  requires a stable sole-command context, a trusted `git-cli worktree remove
  <exact owned absolute managed path>` call or a literal `git -C <exact owned
  non-default managed checkout> merge --no-commit --no-ff --no-edit <verified
  current-main commit>` preparation call. Each must be the only command in its
  tool call, without a preceding source, environment assignment, or wrapper.
  All target, ownership, backup, holder, default-branch, signing, and hook
  guards remain in effect. If an owner tool refuses, stop and report its exact
  error. If a commit-time hook requires
  missing `DEVLOG_LAYOUT`, stop and report the exact error to the environment
  owner.
- Use `devlog new` for durable outcomes. It writes a unique
  `docs/devlog/pending/YYYY-MM-DD-slug.md`; keep its identity comment.
- Ordinary PRs never edit month files, fold entries, or refresh the shared
  index. Merged fragments are immutable; corrections are new entries.
- After each release, the maintainer prepares a dedicated fold PR with
  `scripts/ci/fold-devlog.sh`; it does not commit or push.
- Only canonical `chore/devlog-fold-<date>` maintenance PRs may fold.
- See `docs/devlog/README.md` for authoring and release-fold validation.

## Required Local Check Entrypoints

- Exact check contents and tool prerequisites are defined in
  `docs/runbooks/workspace-maintenance-reference.md`.
- Default local development and pre-PR check:
  `bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast`
- Docs-only changes:
  `bash scripts/ci/nils-cli-checks-entrypoint.sh --docs-only`
- Full CI parity checks are not the default local loop. Run them locally only
  when debugging CI, preparing release-quality verification, or explicitly
  asked:
  `NILS_CLI_TEST_RUNNER=nextest bash scripts/ci/nils-cli-checks-entrypoint.sh`
- Full coverage runs in the nightly, manual and release full-validation
  workflow, not on normal PRs. Run it locally only for coverage maintenance,
  release-quality verification, or CI debugging:
  `NILS_CLI_TEST_RUNNER=nextest bash scripts/ci/nils-cli-checks-entrypoint.sh --with-coverage`
- GitHub required checks for `main` must include `test`, `test_containment`,
  and `cargo-deny (advisories + bans)` after the maintainer applies the migration
  in `docs/runbooks/delivery-and-releases.md`. Full Linux/macOS and coverage are release gates.
- When completion assets change, also run:
  - `zsh -n completions/zsh/_<cli>`
  - `bash -n completions/bash/<cli>`

## Repo Conventions

- In Rust tests, prefer `pretty_assertions::{assert_eq, assert_ne}` for clearer
  diffs.
- New crates use `jiff` for date/time (`jiff = { workspace = true }`); `chrono`
  / `time` are grandfathered in existing crates only. See
  `docs/runbooks/new-cli-crate-development-standard.md`.
- Every user-facing CLI must expose root `-V, --version`.
- For clap-based CLIs, set `#[command(version)]` on the root `Parser`.
- `--help` output should show `-V, --version`.
- Put new code in new modules, not in files listed in
  `scripts/ci/file-size-baseline.tsv`; baselined files cannot grow, not even by
  a `mod` line. Check with `bash scripts/ci/file-size-audit.sh --strict`. See
  the file-size ratchet in `docs/runbooks/workspace-maintenance-reference.md`.

## Local Helpers

- Recommended tooling bootstrap: `scripts/setup-rust-tooling.sh`
- Local release install helper:
  `./scripts/install-local-release-binaries.sh`
