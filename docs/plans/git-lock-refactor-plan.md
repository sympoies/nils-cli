# Plan: Git-Lock Refactor For Maintainability And Coverage

## Overview
Refactor the `git-lock` crate to centralize lock file IO, metadata parsing, and output formatting without changing CLI behavior or the lock-file format. Introduce small, testable abstractions to reduce duplication across command handlers, then expand unit and integration tests to raise coverage. The work is scoped to `crates/git-lock` and its tests; other crates and external behavior remain unchanged.

## Scope
- In scope: `crates/git-lock/src` refactor, `crates/git-lock/tests` additions, new unit tests inside `crates/git-lock/src` modules.
- Out of scope: CLI behavior changes, lock file format changes, new commands, changes to other crates or workspace-wide tooling.

## Assumptions (if any)
1. CLI output strings and exit codes must remain stable to preserve existing behavior and tests.
2. Lock file storage layout and timestamp format must stay identical.
3. No new third-party dependencies are required for the refactor or tests.

## Sprint 0: Baseline And Guardrails
**Goal**: Capture a coverage baseline and set guardrails before changing code.
**Demo/Validation**:
- Command(s): `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`
- Verify: Baseline coverage is captured and workspace coverage stays >= 80.00%.

### Task 0.1: Record Coverage Baseline For git-lock
- **Location**:
  - `docs/plans/git-lock-refactor-plan.md`
- **Description**: Run coverage to capture the current git-lock coverage and record the baseline + target in the PR description or a short note appended to this plan.
- **Dependencies**:
  - none
- **Complexity**: 2
- **Acceptance criteria**:
  - Baseline git-lock line coverage is recorded with a numeric target (baseline +10pp or 85% minimum, whichever is higher).
- **Validation**:
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

Baseline notes (2026-02-02):
- git-lock line coverage: 82.05% (521/635 lines hit; 114 missed).
- Target line coverage: 92.05% (baseline +10pp).

## Sprint 1: Core Abstractions And Command Wiring
**Goal**: Centralize lock storage and metadata parsing, then rewire command handlers to use those abstractions while preserving behavior.
**Demo/Validation**:
- Command(s): `cargo test -p git-lock --tests`
- Verify: Existing integration tests pass without output changes.

### Task 1.1: Introduce LockStore For Paths And Persistence
- **Location**:
  - `crates/git-lock/src/fs.rs`
  - `crates/git-lock/src/store.rs`
  - `crates/git-lock/src/main.rs`
- **Description**: Create a `LockStore` (or `LockPaths`) abstraction that owns the repo id, lock directory path, and helpers for `lock_path`, `latest_path`, `ensure_dir`, `read_latest`, `write_latest`, `read_lock`, and `write_lock`. Move path-building and latest-label resolution out of command modules, keeping `LockFile` parsing in `fs.rs`.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - All path construction and latest-label IO is routed through the new store abstraction.
  - `fs.rs` retains only parsing/time helpers and `LockFile` data structure.
- **Validation**:
  - `cargo test -p git-lock --tests`

### Task 1.2: Add Git Backend Seam And Lock Details Loader
- **Location**:
  - `crates/git-lock/src/git.rs`
  - `crates/git-lock/src/lock_view.rs`
  - `crates/git-lock/src/list.rs`
  - `crates/git-lock/src/copy.rs`
  - `crates/git-lock/src/delete.rs`
  - `crates/git-lock/src/unlock.rs`
- **Description**: Add a small `GitBackend` trait with a default implementation backed by the existing git helpers. Introduce `LockDetails` (label, hash, note, timestamp, subject, epoch) built from `LockStore` + `GitBackend` to remove duplicated parsing and to enable deterministic unit tests.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Copy/delete/list/unlock no longer duplicate lock-file parsing logic.
  - Timestamp parsing and subject lookup are performed in one place and called once per entry.
  - Commands access git subject data through `LockDetails` or `GitBackend`, not ad-hoc git calls.
- **Validation**:
  - `cargo test -p git-lock --tests`

### Task 1.3: Centralize Error/Message Mapping
- **Location**:
  - `crates/git-lock/src/messages.rs`
  - `crates/git-lock/src/main.rs`
  - `crates/git-lock/src/lock.rs`
  - `crates/git-lock/src/unlock.rs`
  - `crates/git-lock/src/list.rs`
  - `crates/git-lock/src/copy.rs`
  - `crates/git-lock/src/delete.rs`
  - `crates/git-lock/src/diff.rs`
  - `crates/git-lock/src/tag.rs`
- **Description**: Create a small message/error mapping module for user-facing strings (usage, not-a-repo, missing lock, etc.) to keep output stable and reduce duplication. Update command modules to use these helpers where appropriate without changing text.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - User-facing error/help strings are centralized and reused.
  - Output text is byte-for-byte identical to existing behavior.
- **Validation**:
  - `cargo test -p git-lock --tests`

### Task 1.4: Rewire Command Handlers To Use Store + Details
- **Location**:
  - `crates/git-lock/src/lock.rs`
  - `crates/git-lock/src/unlock.rs`
  - `crates/git-lock/src/list.rs`
  - `crates/git-lock/src/copy.rs`
  - `crates/git-lock/src/delete.rs`
  - `crates/git-lock/src/diff.rs`
  - `crates/git-lock/src/tag.rs`
- **Description**: Update command handlers to use `LockStore`, `LockDetails`, and `messages` for all IO and metadata access, keeping existing output strings and error handling. Preserve `diff` and `tag` behavior but move any duplicated lock-file parsing into the shared helper.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - No command builds lock paths directly.
  - All existing integration tests pass without changing expected output.
- **Validation**:
  - `cargo test -p git-lock --tests`

## Sprint 2: Testability Seams And Coverage Growth
**Goal**: Add unit tests around parsing and IO seams, then expand integration coverage for missing edge cases.
**Demo/Validation**:
- Command(s): `cargo test -p git-lock`
- Verify: New unit + integration tests pass and cover new helper modules.

### Task 2.1: Unit Tests For Parsing And Store Logic
- **Location**:
  - `crates/git-lock/src/fs.rs`
  - `crates/git-lock/src/store.rs`
  - `crates/git-lock/src/lock_view.rs`
- **Description**: Add unit tests covering `parse_lock_line`, `timestamp_epoch` (valid/invalid), latest-label resolution with empty files, and lock-path construction under both set and unset `ZSH_CACHE_DIR`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Parsing helpers have unit tests for normal and edge inputs.
  - Store behavior for empty or missing latest marker is validated.
  - `LockDetails` uses the `GitBackend` seam and does not invoke git more than once per entry.
- **Validation**:
  - `cargo test -p git-lock --lib`

### Task 2.2: Make Prompt Confirm Testable
- **Location**:
  - `crates/git-lock/src/prompt.rs`
- **Description**: Add a `confirm_with_io` helper that accepts `Read`/`Write` so prompt handling can be unit tested. Keep `confirm` as the public wrapper using stdin/stdout, and add tests for `y`, `Y`, `n`, and empty input cases.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Prompt behavior is covered by unit tests without spawning the CLI.
  - User-visible prompt output remains unchanged.
- **Validation**:
  - `cargo test -p git-lock --lib`

### Task 2.3: Expand Integration Coverage For Missing Edge Cases
- **Location**:
  - `crates/git-lock/tests/edge_cases.rs`
  - `crates/git-lock/tests/copy_delete.rs`
  - `crates/git-lock/tests/diff_tag.rs`
  - `crates/git-lock/tests/lock_unlock.rs`
- **Description**: Add integration tests for missing source labels (copy), delete without label and without latest, diff missing second label, list when lock dir is missing/empty, unlock with non-existent label, tag defaulting to git subject when `-m` is absent, and corrupted lock files (empty first line, invalid timestamp, extra whitespace) using fixture files.
- **Dependencies**:
  - Task 1.4
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - New tests assert key output lines for each edge case.
  - Failure modes exit non-zero where appropriate.
- **Validation**:
  - `cargo test -p git-lock --tests`

### Task 2.4: Coverage Check And Tuning
- **Location**:
  - `crates/git-lock/tests`
  - `crates/git-lock/src`
- **Description**: Run coverage for the workspace and add/adjust tests if git-lock coverage is still low, aiming to lift coverage without overfitting to implementation details.
- **Dependencies**:
  - Task 2.1
  - Task 2.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Coverage run completes and overall workspace line coverage remains >= 80.00%.
  - git-lock module coverage increases compared to baseline.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --workspace`
  - `zsh -f tests/zsh/completion.test.zsh`

## Testing Strategy
- Unit: Add module-level tests for parsing, store/path behavior, and prompt confirmation logic.
- Integration: Extend existing CLI tests to cover missing labels, empty directories, and tag message defaults.
- E2E/manual: Spot-check `git-lock list` and `git-lock unlock` flows in a temp repo after refactor if output changes are suspected.

## Risks & gotchas
- Rewiring command handlers may inadvertently change output ordering or spacing, breaking integration tests.
- Lock dir default behavior when `ZSH_CACHE_DIR` is unset must remain unchanged to avoid path regressions.
- Adding prompt IO seams must not alter stdout flushing or abort messaging.

## Rollback plan
- Revert `crates/git-lock` to the pre-refactor commit if output compatibility issues are found.
- Keep new tests; if failures are due to behavior changes, gate them behind the restored implementation before reattempting refactor.
