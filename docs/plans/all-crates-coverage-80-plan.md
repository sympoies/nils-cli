# Plan: Raise every crate to at least 80% line coverage

## Overview
This plan raises per-crate line coverage so every crate in the workspace is at or above 80.00%. The current coverage snapshot shows three crates below the threshold: `cli-template` (62.07%), `git-summary` (72.73%), and `plan-tooling` (79.76%). The implementation strategy is test-first and behavior-preserving: add deterministic tests for untested CLI paths and validation branches, then rerun workspace coverage and gate checks.

## Scope
- In scope:
  - Add/extend tests in `cli-template`, `git-summary`, and `plan-tooling` only.
  - Keep runtime behavior unchanged (no feature changes).
  - Validate with crate tests, required repo checks, and workspace coverage.
- Out of scope:
  - Refactoring unrelated crates.
  - Changing CLI output contracts for existing commands.
  - Modifying CI workflow definitions.

## Assumptions (if any)
1. Coverage is measured via `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`.
2. Per-crate line coverage is computed from `target/coverage/lcov.info` grouped by `crates/<name>/` paths.
3. Existing tests and fixtures in each target crate can be reused without adding new external dependencies.

## Sprint 1: Lift low-coverage crates with targeted tests
**Goal**: Raise `cli-template`, `git-summary`, and `plan-tooling` to >= 80% line coverage.
**Demo/Validation**:
- Command(s):
  - `cargo test -p cli-template -p git-summary -p plan-tooling`
- Verify:
  - New tests pass and cover previously unexecuted command branches.

### Task 1.1: Cover `cli-template` progress and logging fallback paths
- **Location**:
  - `crates/cli-template/tests/cli.rs`
- **Description**: Add integration tests to exercise `progress-demo` execution and invalid `--log-level` fallback while preserving current user-facing output behavior.
- **Dependencies**:
  - none
- **Complexity**: 2
- **Acceptance criteria**:
  - `progress-demo` test asserts successful exit and expected `done` output.
  - Invalid `--log-level` test confirms command still succeeds and returns expected greeting output.
- **Validation**:
  - `cargo test -p cli-template`

### Task 1.2: Cover `git-summary` help and date-shortcut command branches
- **Location**:
  - `crates/git-summary/tests/cli_paths.rs`
  - `crates/git-summary/tests/edge_cases.rs`
- **Description**: Add integration tests for no-arg/help output and each date shortcut command branch (`today`, `yesterday`, `this-month`, `last-month`, `this-week`, `last-week`, plus `all`) using deterministic assertions on header/usage text.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Help/no-arg invocations return success and print usage sections.
  - Shortcut commands execute successfully in a temp git repo and emit expected `Git summary` headers.
- **Validation**:
  - `cargo test -p git-summary`

### Task 1.3: Cover `plan-tooling` `to-json` usage/error branches
- **Location**:
  - `crates/plan-tooling/tests/to_json.rs`
- **Description**: Add focused CLI tests for `to-json` usage paths (for example `--help`, unknown flags, and missing option values) so parser entrypoints and usage rendering are exercised.
- **Dependencies**:
  - none
- **Complexity**: 2
- **Acceptance criteria**:
  - New tests assert expected exit codes for usage errors.
  - `--help` path is validated for expected usage text.
- **Validation**:
  - `cargo test -p plan-tooling`

## Sprint 2: Validate workspace gates and publish feature PR
**Goal**: Prove all crates are >= 80% and publish changes as a feature PR.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - All mandatory checks pass.
  - Per-crate coverage report shows no crate below 80%.

### Task 2.1: Run required checks and verify per-crate coverage thresholds
- **Location**:
  - `target/coverage/lcov.info`
- **Description**: Execute required repo checks and recompute per-crate coverage percentages from lcov output to confirm threshold compliance.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Required checks are green.
  - Coverage summary confirms all crates >= 80.00%.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`

### Task 2.2: Commit and open feature PR
- **Location**:
  - `references/PR_TEMPLATE.md`
- **Description**: Create a feature branch, commit with `semantic-commit-autostage`, push, and open a PR with testing evidence and coverage results.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Commit message follows semantic commit format.
  - PR is created with summary, changes, testing, and risk sections.
- **Validation**:
  - `gh pr view --json number,title,url`

## Testing Strategy
- Unit/integration:
  - Crate-specific tests for `cli-template`, `git-summary`, `plan-tooling`.
- Workspace checks:
  - Run required fmt/clippy/test/zsh checks via `nils-cli-verify-required-checks`.
- Coverage:
  - Re-generate lcov and verify both total and per-crate thresholds.

## Risks & gotchas
- Date-based commands in `git-summary` can be flaky if assertions depend on exact dates; tests must assert stable labels/prefixes only.
- CLI help formatting can shift with clap updates; assertions should target durable substrings rather than full snapshots.
- `progress-demo` includes sleep calls; avoid overly strict timing assertions.
- `gh pr create` may fail in environments without GitHub auth; if so, retain branch/commit and report exact blocker.

## Rollback plan
- Revert only newly added test files/blocks if they introduce flakiness.
- Keep functional code unchanged; rollback scope should be test-only.
- If coverage remains below target, split additional test-only follow-up tasks per crate instead of broad refactors.
