# Plan: Repo root detection audit + fix (all CLIs)

## Overview
This plan fixes repo-root detection across this workspace so that repo-relative paths are resolved from the Git top-level for the current working directory (when inside a Git work tree), with a fallback to the current working directory when Git isn’t available or the directory isn’t a work tree. The immediate driver is a `plan-tooling` bug where repo-relative `--file docs/plans/...` arguments can fail to resolve due to non-cwd-based repo-root discovery. The plan also audits every CLI binary in this repo to confirm whether they share the same failure mode and adds regression tests to prevent reintroduction.

## Scope
- In scope:
  - Audit repo-root detection and repo-relative path resolution for every workspace CLI binary:
    - `api-gql`, `api-rest`, `api-test`, `cli-template`, `fzf-cli`, `git-lock`, `git-scope`, `git-summary`, `image-processing`, `plan-tooling`, `semantic-commit`
  - Fix `plan-tooling` to determine repo root using the current working directory’s Git top-level (fallback: current working directory).
  - Add regression tests for `plan-tooling` (and any other affected CLIs discovered during the audit).
  - Update docs/specs where they describe repo-relative path behavior.
- Out of scope:
  - Changing CLI behavior unrelated to repo-root discovery (output formatting, flags, error messages), except where necessary to keep path resolution consistent.
  - Introducing a new configuration/env-var override mechanism for repo-root discovery.

## Assumptions (if any)
1. Git is available in CI and is already a required runtime dependency for Git-centric CLIs.
2. For commands that accept repo-relative paths, “repo root” means Git top-level for the current working directory (not the directory where the binary is installed).
3. Some CLIs may intentionally require being inside a Git work tree; this plan keeps that requirement unless a spec explicitly says otherwise.

## Sprint 1: Inventory + contract
**Goal**: Make repo-root behavior explicit and identify which CLIs are affected.
**Demo/Validation**:
- Command(s): `rg -n "show-toplevel|\\.git" crates -S`, `rg -n "repo_root" crates -S`
- Verify: Inventory doc exists and lists every CLI binary with its repo-root strategy and any repo-relative path assumptions.

### Task 1.1: Write repo-root contract + inventory doc
- **Location**:
  - `docs/repo-root-resolution.md`
- **Description**: Add a short doc that defines the repo-root discovery algorithm and documents, per CLI, (a) whether it uses Git top-level / `.git` upward search / other, and (b) whether it resolves any arguments as repo-relative. This doc is the audit output and should be updated again after Sprint 2/3 changes.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - `docs/repo-root-resolution.md` exists.
  - The doc lists all workspace CLI binaries and identifies any that accept repo-relative file/dir arguments.
  - The doc explicitly states the repo-root algorithm: “Git top-level for cwd; fallback to cwd”.
- **Validation**:
  - `test -f docs/repo-root-resolution.md`
  - `rg -n "Git top-level" docs/repo-root-resolution.md`

## Sprint 2: Fix `plan-tooling` and add regression coverage
**Goal**: `plan-tooling` resolves repo-relative paths from Git top-level of the current working directory and is resilient to environment-based repo-root overrides.
**Demo/Validation**:
- Command(s):
  - `cargo test -p plan-tooling`
  - `cargo run -q -p plan-tooling -- validate --file docs/plans/repo-root-detection-audit-fix-plan.md`
  - `mkdir -p .tmp/pt && (cd .tmp/pt && cargo run -q -p plan-tooling -- validate --file docs/plans/repo-root-detection-audit-fix-plan.md)`
- Verify:
  - All `plan-tooling` tests pass.
  - Running `plan-tooling validate --file docs/plans/...` works from both repo root and a nested directory within the repo.

### Task 2.1: Update `plan-tooling` repo-root detection to use Git top-level (fallback: cwd)
- **Location**:
  - `crates/plan-tooling/src/repo_root.rs`
- **Description**: Replace repo-root detection so it first tries `git rev-parse --show-toplevel` for the process current working directory. If that succeeds, return the Git top-level path. Otherwise, return `std::env::current_dir()` (or `.` on error). Do not consult any environment variable to override repo-root detection.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Repo root equals Git top-level when run anywhere inside a Git work tree.
  - Outside a Git work tree, repo root equals the current working directory (no panic).
  - Repo-relative `--file` paths in `validate`, `to-json`, `batches`, and `scaffold` resolve against Git top-level when available.
- **Validation**:
  - `cargo test -p plan-tooling`
  - `cargo run -q -p plan-tooling -- validate --file docs/plans/repo-root-detection-audit-fix-plan.md`

### Task 2.2: Update `plan-tooling` tests to remove env-dependent repo-root assumptions
- **Location**:
  - `crates/plan-tooling/tests/common.rs`
- **Description**: Update the test harness so repo-root discovery depends only on the temp repo’s working directory (and Git), not on any environment variable. This should make tests match real invocation behavior.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo test -p plan-tooling` passes without requiring any special environment configuration.
  - Test harness sets the working directory to the temp repo root (or a nested path) to exercise Git top-level detection.
- **Validation**:
  - `cargo test -p plan-tooling`

### Task 2.3: Add a regression test for “ignore env override, use Git top-level”
- **Location**:
  - `crates/plan-tooling/tests/validate.rs`
- **Description**: Add a regression test that sets the legacy repo-root override environment variable to a non-repo directory, then runs `plan-tooling validate --file docs/plans/example-plan.md` from inside a temp Git repo and asserts success. This locks the behavior that repo-root discovery is based on cwd + Git, not environment.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - The new test fails on the pre-fix behavior and passes after Task 2.1.
  - The test uses a repo-relative `--file docs/plans/...` path to ensure the correct root is used.
- **Validation**:
  - `cargo test -p plan-tooling`

## Sprint 3: Sweep all other CLIs and align where needed
**Goal**: Confirm no other workspace CLI has the same repo-root failure mode; apply fixes only where necessary and add targeted regression tests.
**Demo/Validation**:
- Command(s): `cargo test --workspace`, `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: CI-equivalent checks pass; inventory doc reflects final behavior.

### Task 3.1: Audit each CLI crate for repo-root + repo-relative path semantics
- **Location**:
  - `crates/api-gql/src/main.rs`
  - `crates/api-rest/src/main.rs`
  - `crates/api-testing-core/src/suite/resolve.rs`
  - `crates/api-test/src/main.rs`
  - `crates/cli-template/src/main.rs`
  - `crates/fzf-cli/src/main.rs`
  - `crates/fzf-cli/src/util.rs`
  - `crates/fzf-cli/src/git_commit.rs`
  - `crates/fzf-cli/src/open.rs`
  - `crates/git-lock/src/main.rs`
  - `crates/git-lock/src/fs.rs`
  - `crates/git-lock/src/git.rs`
  - `crates/git-scope/src/main.rs`
  - `crates/git-scope/src/git.rs`
  - `crates/git-summary/src/main.rs`
  - `crates/image-processing/src/util.rs`
  - `crates/semantic-commit/src/main.rs`
  - `crates/semantic-commit/src/git.rs`
  - `crates/semantic-commit/src/staged_context.rs`
  - `docs/repo-root-resolution.md`
- **Description**: For each CLI, verify how repo root is determined (if at all) and whether any arguments are treated as repo-relative. If any CLI computes repo root in a way that is not based on cwd + Git top-level (when applicable), update it to match the contract from Sprint 1. Update `docs/repo-root-resolution.md` with findings and post-fix state.
- **Dependencies**:
  - Task 1.1
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - `docs/repo-root-resolution.md` is updated with final, post-fix behavior for every CLI.
  - Any CLI found with env-dependent repo-root discovery (or cwd-only discovery while treating paths as repo-relative) is corrected.
  - Any corrected CLI has at least one regression test covering invocation from a nested directory within a temp Git repo.
- **Validation**:
  - `cargo test --workspace`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit:
  - `plan-tooling` regression test for ignoring env-based overrides.
  - `plan-tooling` tests for nested-directory invocation resolving repo-relative `--file` paths.
- Integration:
  - Run `plan-tooling` commands from repo root and a nested directory and confirm consistent behavior.
- E2E/manual:
  - Confirm `plan-tooling validate --file docs/plans/...` works when invoked via an installed binary and via `cargo run`, from both repo root and nested directories.

## Risks & gotchas
- Some existing workflows may have (implicitly) relied on env-based repo-root overrides; removing that is a behavior change. Mitigation: document the new contract and add clear regression tests.
- Git worktree setups may use `.git` as a file rather than a directory; Git top-level detection via `git rev-parse --show-toplevel` avoids this pitfall.
- If a CLI intentionally operates outside Git repositories, ensure the fallback-to-cwd behavior is safe and does not create surprising repo-relative resolution.

## Rollback plan
- Revert `crates/plan-tooling/src/repo_root.rs` to the previous behavior and remove the new regression test(s).
- Restore the previous `crates/plan-tooling/tests/common.rs` harness behavior if needed.
- Keep `docs/repo-root-resolution.md` updated to match whichever behavior is rolled back to avoid operator confusion.
