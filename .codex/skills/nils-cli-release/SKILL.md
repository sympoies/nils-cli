---
name: nils-cli-release
description: Bump CLI versions and tag a release to trigger the CI flow.
---

# Nils CLI Release

## Contract

Prereqs:

- Run inside the `nils-cli` git work tree (the script resolves the repo root via `git`).
- `git`, `python3`, `cargo`, `semantic-commit`, and `git-scope` available on `PATH`.
- `cargo-nextest` available on `PATH` when using default release checks (`NILS_CLI_TEST_RUNNER=nextest`).
- Release checks available at `.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh` (unless `--skip-checks`).

Inputs:

- Required:
  - `--version X.Y.Z` (accepts `vX.Y.Z` and normalizes to `X.Y.Z`)
- Optional:
  - `--skip-checks` (skip full lint/tests; still runs `cargo check --workspace` to refresh `Cargo.lock`)
  - `--skip-readme` (do not update README release tag example)
  - `--skip-push` (do not push commit or tag to `origin`)
  - `--allow-dirty` (allow a dirty working tree)
  - `--force-tag` (delete existing local/remote tag before re-tagging)
  - `NILS_CLI_TEST_RUNNER=cargo|nextest` (environment variable; default is `nextest` in this release script)

Outputs:

- Updates workspace version in `Cargo.toml` and any crate `Cargo.toml` files with explicit `version = "..."`.
- Updates README release tag examples (unless `--skip-readme`).
- Refreshes `Cargo.lock` via `cargo check` or the full checks script.
- Runs release checks through `nils-cli-checks.sh` with `NILS_CLI_TEST_RUNNER=nextest` by default (unless overridden).
- Creates a semantic commit for the version bump.
- Creates an annotated tag `vX.Y.Z` and (unless `--skip-push`) pushes commit + tag to `origin`.
- GitHub Release artifacts are built by `.github/workflows/release.yml` and include all workspace `bin` targets (auto-discovered via `scripts/workspace-bins.py`).

Exit codes:

- `0`: success
- `1`: command failed or a prerequisite is missing
- `2`: usage error or invalid inputs

Failure modes:

- Invalid version format or missing `--version`.
- Dirty working tree without `--allow-dirty`.
- Tag already exists without `--force-tag`.
- Required commands missing (`git`, `python3`, `cargo`, `semantic-commit`, `git-scope`).
- `cargo-nextest` missing while default check path (`nextest`) is active.
- Release checks or `cargo check` fail.
- Commit or tag creation fails.

## Scripts (only entrypoints)

- `.codex/skills/nils-cli-release/scripts/nils-cli-release.sh`

## Workflow

- Validate inputs and environment.
- Bump workspace + crate versions and update README.
- Run checks (defaulting to `nextest`, or `cargo check` with `--skip-checks`) to refresh `Cargo.lock`.
- Commit with `semantic-commit`, tag `vX.Y.Z`, and push to trigger the release workflow.
