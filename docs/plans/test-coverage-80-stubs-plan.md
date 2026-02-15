# Plan: Raise workspace line coverage to 80% (stub/fixture-first)

## Overview
This plan raises Rust workspace **total line coverage** from **75.01%** (13886/18513 lines hit; from `target/coverage/lcov.info`) to **at least 80.00%** by prioritizing **stable, hermetic test stubs/fixtures** over raw test count. The main lever is to build reusable fixtures (stub binaries, temp repo builders, and loopback HTTP servers) so that the largest low-coverage modules can be exercised deterministically without relying on external network access or non-stubbed system tools. CI coverage gating will be raised to 80% only after the new tests reliably keep the workspace above the threshold with a small buffer.

## Scope
- In scope:
  - Create a reusable stub/fixture toolkit for tests (PATH-scoped stub binaries, env/cwd guards, loopback HTTP server, git repo builder).
  - Add deterministic unit and integration tests targeting the biggest uncovered-line hotspots.
  - Minor refactors strictly for testability when a boundary must be injected (e.g., extract a pure helper, isolate filesystem/network calls).
  - Raise CI coverage gate to 80% after coverage is achieved and stable.
- Out of scope:
  - Feature changes to CLI behavior/output unrelated to testability.
  - Adding new CI runtime requirements on external services or globally installed binaries.
  - Large refactors that change architecture or public APIs beyond what tests require.

## Assumptions (if any)
1. Coverage is measured and enforced via the existing workflow:
   - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
   - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
2. Tests must be hermetic:
   - No real network access: use loopback servers only.
   - No reliance on developer machine binaries beyond what is explicitly stubbed via `PATH`.
   - No reliance on global state: use temp dirs + env guards.
3. Current top uncovered-line hotspots (approx; will shift as code changes) include:
   - `crates/api-testing-core/src/suite/runner.rs`
   - `crates/api-testing-core/src/suite/cleanup.rs`
   - `crates/api-testing-core/src/suite/auth.rs`
   - `crates/fzf-cli/src/git_commit.rs`
   - `crates/git-lock/src/copy.rs`
4. From a baseline of 75.01% on 18513 lines, reaching 80.00% requires ~**925** additional lines hit (exact delta varies with code movement and new helpers).

## Acceptance criteria
- `scripts/ci/coverage-summary.sh target/coverage/lcov.info` reports **Total line coverage >= 80.00%** on CI.
- Required repo checks pass:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Tests are deterministic and do not depend on external network or non-stubbed system binaries.
- CI coverage gate is raised to 80% (and stays green on the PR).

## Validation
- `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
- `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Sprint 1: Build a stable stub + fixture toolkit
**Goal**: Create reusable, hermetic test infrastructure so later coverage work is fast, parallelizable, and non-flaky.
**Demo/Validation**:
- Command(s):
  - `cargo test -p api-testing-core -p image-processing -p fzf-cli -p git-lock`
- Verify:
  - Tests can run without network access.
  - Stub binaries are used via `PATH` and do not require system tools.

**Parallel lanes**:
- Lane A: Task 1.1
- Lane B: Task 1.2
- Lane C: Task 1.3
- Lane D: Task 1.4

### Task 1.1: Add a shared test-support crate for env/cwd/PATH stubbing
- **Location**:
  - `crates/nils-test-support/Cargo.toml`
  - `crates/nils-test-support/src/lib.rs`
  - `Cargo.toml` (workspace members)
- **Description**: Add a small Rust crate intended for **dev-dependency only** that provides:
  - `EnvGuard` (set/unset vars; restore on drop)
  - `CwdGuard` (set current dir; restore on drop)
  - `StubBinDir` + `write_exe()` helpers (create temp `bin/`, write executable scripts, return `PATH` fragment)
  - `GlobalStateLock` (a single process-wide mutex guard) for any test that must mutate process-global state
  - Minimal, stable APIs to avoid churn; keep it small to avoid inflating the coverage denominator.
- **Dependencies**: none
- **Complexity**: 7
- **Acceptance criteria**:
  - At least one existing crate test uses `nils-test-support` instead of local ad-hoc helpers.
  - No global state leakage: env and cwd are restored even on test failure.
  - Any helper that mutates env/cwd/PATH requires holding `GlobalStateLock` (documented and used by tests).
- **Validation**:
  - `cargo test -p nils-test-support`
  - `cargo test -p image-processing` (as a consumer)

### Task 1.2: Standardize stub-binary scripts (no external tools required)
- **Location**:
  - `crates/nils-test-support/src/stubs.rs`
  - Update existing per-crate test helpers:
    - `crates/image-processing/tests/common.rs`
    - `crates/fzf-cli/tests/common.rs`
- **Description**: Centralize commonly used stub scripts so tests do not rely on system binaries:
  - `magick`, `convert`, `identify` (ImageMagick)
  - `fzf` (deterministic selection/output + exit code)
  - `bat` (preview; can be no-op)
  - `tree`, `file` (for git-scope behavior)
  - Ensure stubs are POSIX-friendly (prefer `/bin/bash` only when necessary) and safe on macOS + Ubuntu.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests can force “tool present” vs “tool missing” by PATH ordering only.
  - Stubs can log invocations to a temp file so tests can assert the stub was actually called.
  - No stub executes network I/O.
- **Validation**:
  - `cargo test -p image-processing`
  - `cargo test -p fzf-cli`

### Task 1.3: Add a loopback HTTP test server fixture (no real network)
- **Location**:
  - `crates/nils-test-support/src/http.rs`
  - (Optional) `crates/api-testing-core/src/http.rs` refactor to allow injection in tests
- **Description**: Implement a small loopback HTTP server fixture for tests that need HTTP behavior without external network:
  - Bind `127.0.0.1:0` to avoid port conflicts.
  - Allow registering fixed responses per method+path, with JSON bodies, status codes, and headers.
  - Capture request summaries for assertions (method, path, minimal body).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - A new test can run a REST/GraphQL flow against the loopback server with deterministic assertions.
  - No sleeps/timeouts required in tests (server starts before returning its base URL).
  - Server shutdown is deterministic (no hangs on drop; threads join cleanly).
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 1.4: Add fixture builders for setup directories and suite files
- **Location**:
  - `crates/nils-test-support/src/fixtures.rs`
- **Description**: Provide helpers to build minimal but realistic on-disk fixtures in temp dirs:
  - `setup/rest/endpoints(.local).env`, `tokens(.local).env`
  - `setup/graphql/endpoints(.local).env`, `jwts(.local).env`, `schema.env`, schema file(s)
  - Minimal suite manifests used by `api-testing-core` runner/cleanup (file layout mirrors real usage).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Fixtures are created with explicit file contents (no reliance on repo-local files outside the temp dir).
  - Helpers document expected file precedence behavior used by tests.
- **Validation**:
  - `cargo test -p api-rest -p api-gql -p api-testing-core`

## Sprint 2: Lift api-testing-core coverage using hermetic fixtures
**Goal**: Reduce the largest uncovered-line hotspot by covering suite runner/cleanup/auth logic without external network or real service dependencies.
**Demo/Validation**:
- Command(s):
  - `cargo test -p api-testing-core`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - Coverage increases measurably in:
    - `crates/api-testing-core/src/suite/runner.rs`
    - `crates/api-testing-core/src/suite/cleanup.rs`
    - `crates/api-testing-core/src/suite/auth.rs`
  - Workspace coverage checkpoint: **>= 78.00%** after Sprint 2 lands (measured by `coverage-summary.sh`).

**Parallel lanes**:
- Lane A: Task 2.1
- Lane B: Task 2.2
- Lane C: Task 2.3
- Lane D: Task 2.4 (after Task 1.3 is available)

### Task 2.1: Add unit tests for suite runner helpers and precedence rules
- **Location**:
  - `crates/api-testing-core/src/suite/runner.rs`
- **Description**: Add unit tests covering (at minimum):
  - Argument masking in `mask_args_for_command_snippet` (both `--token value` and `--token=value` forms).
  - `sanitize_id` normalization edge cases (empty, punctuation, unicode).
  - REST/GraphQL URL resolution precedence (`override > suite defaults > env var > env file lookup > fallback`).
  - Token profile resolution from `setup/rest/tokens(.local).env`.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests do not perform HTTP requests.
  - Tests assert stable substrings and deterministic outputs.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.2: Add unit tests for cleanup parsing/validation and failure diagnostics
- **Location**:
  - `crates/api-testing-core/src/suite/cleanup.rs`
- **Description**: Add unit tests that hit both success and early-failure paths:
  - Template variable substitution and `varsTemplate` parsing failures.
  - Invalid/missing fields in cleanup steps and how errors are reported.
  - Behavior differences between REST vs GraphQL cleanup steps (type mapping, URL selection).
- **Dependencies**:
  - Task 1.4
- **Complexity**: 8
- **Acceptance criteria**:
  - Failures are asserted by stable error messages (key substrings), not full snapshots.
  - No tests require a real service or external binaries.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.3: Add unit tests for suite auth manager selection and caching behavior
- **Location**:
  - `crates/api-testing-core/src/suite/auth.rs`
- **Description**: Add tests covering:
  - Provider inference/validation (`canonical_provider`).
  - Initialization from suite manifest (missing required secret env vs optional).
  - Token caching semantics (token reuse; error memoization) without performing network.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover both “auth disabled” and “auth configured” branches.
  - No global env state leaks (use `EnvGuard`).
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.4: Add one end-to-end suite run against a loopback server
- **Location**:
  - `crates/api-testing-core/tests/suite_runner_loopback.rs`
- **Description**: Create a minimal suite fixture that:
  - Starts a loopback server (Task 1.3) that serves deterministic REST and/or GraphQL responses.
  - Runs a minimal suite with `fail_fast=false` and asserts summary counts, output files, and cleanup behavior.
  - Ensures “writes disabled” and “no history” options propagate as expected without touching the network.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
- **Complexity**: 9
- **Acceptance criteria**:
  - Suite run completes deterministically under CI.
  - No flakiness from ports/time; server lifecycle is fully owned by the test.
- **Validation**:
  - `cargo test -p api-testing-core`

## Sprint 3: Lift remaining low-coverage CLI crates using stubs and temp repos
**Goal**: Improve coverage in the next-lowest modules (git-lock, fzf-cli, git-scope, git-summary) with deterministic, fixture-driven tests.
**Demo/Validation**:
- Command(s):
  - `cargo test -p git-lock -p fzf-cli -p git-scope -p git-summary`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - Coverage increases in `git-lock/src/copy.rs`, `git-lock/src/tag.rs`, and `fzf-cli` command paths.
  - Workspace coverage checkpoint: **>= 80.50%** before raising the CI gate to 80% (buffer against drift).

**Parallel lanes**:
- Lane A: Task 3.1
- Lane B: Task 3.2
- Lane C: Task 3.3
- Lane D: Task 3.4

### Task 3.1: Add git-lock integration tests for copy/tag paths with prompt control
- **Location**:
  - `crates/git-lock/tests/copy_and_tag.rs`
- **Description**: Add tests that:
  - Set `ZSH_CACHE_DIR` to a temp dir so lock files never touch `/git-locks`.
  - Build a temp git repo (real `git init`) and generate lock files.
  - Exercise overwrite prompts by piping `y\n` / `n\n` into the binary under test.
  - Assert exit codes and stable output substrings.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Tests run without requiring any state outside the temp dir.
  - Prompt behavior is covered for both accept and abort.
  - Repo creation does not depend on host git config (set `user.name/email`, disable GPG signing, isolate HOME).
- **Validation**:
  - `cargo test -p git-lock`

### Task 3.2: Expand fzf-cli tests for open/file behaviors using stub fzf + stub open tools
- **Location**:
  - `crates/fzf-cli/src/open.rs`
  - `crates/fzf-cli/src/file.rs`
  - `crates/fzf-cli/tests/open_and_file.rs`
- **Description**: Add tests that:
  - Use stub `fzf` output to deterministically select a file/line.
  - Stub “open” command execution so tests do not launch real apps.
  - Cover `FZF_FILE_MAX_DEPTH` behavior and `.git` filtering.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - No tests open windows or require interactive tools.
  - Selection and resulting “open” invocation are asserted deterministically.
- **Validation**:
  - `cargo test -p fzf-cli`

### Task 3.3: Expand fzf-cli git_commit tests to cover error and success paths
- **Location**:
  - `crates/fzf-cli/src/git_commit.rs`
  - `crates/fzf-cli/tests/git_commit.rs`
- **Description**: Add tests that:
  - Build a temp git repo with multiple commits.
  - Use stub `fzf` to select commits and validate derived command behavior.
  - Cover at least one failure path (e.g., nonzero fzf exit) and one success path.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Tests use temp repos and do not depend on host git config.
  - Output/exit codes match expectations without brittle snapshots.
- **Validation**:
  - `cargo test -p fzf-cli`

### Task 3.4: Add git-scope and git-summary tests for external-tool degradation paths
- **Location**:
  - `crates/git-scope/tests/tool_degradation.rs`
  - `crates/git-summary/tests/cli_paths.rs`
- **Description**: Add tests that:
  - Force `tree` and `file` present/missing via PATH (stubbed), and assert warnings and fallbacks.
  - Cover formatting branches for `--no-color` and stable output ordering.
  - Use temp repos with deterministic commit history for `git-summary`.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests are deterministic across macOS + Ubuntu.
  - Tool-missing behavior matches documented CLI degradation paths.
- **Validation**:
  - `cargo test -p git-scope -p git-summary`

## Sprint 4: Raise the CI coverage gate to 80% and lock it in
**Goal**: Enforce the new coverage baseline via CI once a fresh coverage run confirms >= 80%, and keep the developer workflow clear and reproducible.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`
- Verify:
  - Coverage summary reports >= 80.00% before the gate is raised.
  - CI is green with the new gate after the change is merged.

### Task 4.0: Recompute workspace coverage and confirm gate readiness
- **Location**:
  - `scripts/ci/coverage-summary.sh`
  - `target/coverage/lcov.info` (generated)
- **Description**: Run a fresh coverage pass (recommended after Sprint 2–3 tests land) and record the total line coverage. If coverage is below 80%, capture the gap (lines needed) and document the top 5 lowest-coverage files in `notes/coverage-gap.md` to guide remediation before raising the CI gate.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Coverage summary is generated from a fresh run.
  - If coverage < 80.00%, the gap and top 5 low-coverage files are recorded in `notes/coverage-gap.md` and the gate-raising task is deferred.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - If coverage < 80.00%: `test -s notes/coverage-gap.md`

### Task 4.1: Raise CI coverage threshold from 75% to 80%
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: Change the coverage job env var to:
  - `COVERAGE_FAIL_UNDER_LINES: "80"`
  - Keep this change last to avoid blocking intermediate PRs while test harness is still being built.
- **Dependencies**:
  - Task 4.0
- **Complexity**: 3
- **Acceptance criteria**:
  - Coverage is >= 80.00% on the latest run prior to raising the gate.
  - CI coverage job passes with the new gate.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

### Task 4.2: Update developer docs to reflect the 80% coverage requirement
- **Location**:
  - `DEVELOPMENT.md`
- **Description**: Document:
  - The coverage command used in CI.
  - The minimum supported coverage threshold (80%).
  - A short troubleshooting note (how to identify hotspots from `target/coverage/lcov.info`).
- **Dependencies**:
  - Task 4.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Docs match CI behavior and are runnable verbatim (80% minimum).
- **Validation**:
  - `rg \"COVERAGE_FAIL_UNDER_LINES\" -n .github/workflows/ci.yml`
  - `rg \"80.00%\" -n DEVELOPMENT.md`

### Task 4.3: Close remaining coverage gap (conditional)
- **Location**:
  - `notes/coverage-gap.md`
  - `crates/api-testing-core/src/suite/runner.rs`
  - `crates/api-testing-core/src/suite/cleanup.rs`
  - `crates/api-testing-core/src/suite/auth.rs`
  - `crates/fzf-cli/src/git_commit.rs`
  - `crates/git-lock/src/copy.rs`
- **Description**: If Task 4.0 reports coverage < 80%, add targeted, deterministic tests for the top offenders listed in `notes/coverage-gap.md` using the existing fixture/stub toolkit. Prefer unit tests for pure logic and small integration tests for CLI pathways.
- **Dependencies**:
  - Task 4.0
- **Complexity**: 8
- **Acceptance criteria**:
  - Coverage summary reaches >= 80.00% on a fresh run.
  - Tests are hermetic (no external network; stubbed binaries only).
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Testing Strategy
- Unit:
  - Prefer testing pure helpers (parsers, precedence logic, formatting) directly.
  - Use guards (`EnvGuard`, `CwdGuard`) to avoid global state leakage.
- Integration:
  - Use temp dirs for `setup/` fixtures and “real” file layouts.
  - Use PATH-scoped stub binaries to avoid external tooling requirements.
- E2E/manual:
  - Avoid unless necessary for parity; prefer deterministic command-level tests with stubbed dependencies.

## Risks & gotchas
- Adding a new test-support crate increases the coverage denominator:
  - Mitigation: keep it small and ensure it is executed by many tests (or add direct unit tests in the crate).
- Current `target/coverage/lcov.info` reports **75.01%** total line coverage:
  - Mitigation: run Task 4.0 to confirm the true current coverage, then execute missing test tasks from Sprints 2–3 (or targeted hotspot tests) before raising the CI gate.
- Flaky tests from ports/timeouts:
  - Mitigation: bind to `127.0.0.1:0`, avoid sleeps, and fully own server thread lifecycle.
- Cross-platform executable bit issues:
  - Mitigation: gate stub-exe tests with `#[cfg(unix)]` if needed; CI runs on Ubuntu.
- Parallel work touching the same files:
  - Mitigation: prefer new test files and per-crate lane separation; keep refactors small.

## Rollback plan
- If CI becomes unstable:
  - Temporarily revert CI coverage gate to 75% (`.github/workflows/ci.yml`) while keeping the new tests, then re-raise to 80% after fixing flakiness.
- If a stub/fixture abstraction causes churn:
  - Revert to per-crate `tests/common.rs` helpers and keep the fixture scripts local to each crate.
- If a small number of tests are flaky under CI scheduling:
  - Temporarily mark those tests `#[ignore]` and run them in a separate, non-gating job while fixing determinism issues.
