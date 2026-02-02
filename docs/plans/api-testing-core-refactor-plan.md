# Plan: api-testing-core suite refactor

## Overview
This plan refactors the `api-testing-core` crate with a focus on the `suite` runtime, especially `suite/runner` and `suite/cleanup`, to improve readability and maintainability. It centralizes duplicated helper logic, decomposes large modules into cohesive submodules, and adds characterization tests to preserve behavior. External behavior (CLI output, file layout, warnings, and JSON formats) remains unchanged. Work is staged to keep diffs reviewable and risk low.

## Scope
- In scope: internal module reorganization, shared helper extraction, new unit/integration tests that lock current behavior, minor naming improvements for clarity.
- Out of scope: new features, performance optimizations, CLI flag changes, or changing on-disk output formats.

## Assumptions (if any)
1. Behavioral parity is required; existing tests are authoritative for expected outputs.
2. Adding `#[cfg(test)]` unit tests inside crate modules is acceptable.
3. Refactors will be delivered in small steps so regressions are easy to spot and revert.

## Sprint 1: Shared helpers and characterization tests
**Goal**: Remove duplicated helper logic between `runner` and `cleanup`, and lock key behaviors with tests.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core`
- Verify: tests pass and `suite_runner_loopback` still validates stdout/stderr content and command snippets.

### Task 1.1: Create shared suite runtime helpers
- **Location**:
  - `crates/api-testing-core/src/suite/runtime.rs`
  - `crates/api-testing-core/src/suite/mod.rs`
- **Description**: Introduce a `suite::runtime` module and move shared helpers from `suite/runner.rs` and `suite/cleanup.rs` into it (URL resolution with defaults/env, token profile resolution, GraphQL bearer token resolution, time/ID/path helpers). Export them as `pub(crate)`.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - `suite/runner.rs` and `suite/cleanup.rs` no longer define duplicate helpers.
  - `suite::runtime` provides the shared API surface with clear, single-responsibility functions.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 1.2: Update runner/cleanup to use shared helpers
- **Location**:
  - `crates/api-testing-core/src/suite/runner.rs`
  - `crates/api-testing-core/src/suite/cleanup.rs`
- **Description**: Replace local helper calls with `suite::runtime` usage and remove redundant logic. Keep signatures and error messages identical.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - No behavior changes in `run_suite` and `run_case_cleanup` outputs for existing tests.
  - Error messages and logging remain byte-for-byte identical for common paths.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 1.3: Add in-module characterization tests for shared helpers
- **Location**:
  - `crates/api-testing-core/src/suite/runtime.rs`
  - `crates/api-testing-core/src/suite/runtime_tests.rs`
- **Description**: Add `#[cfg(test)]` unit tests covering URL precedence (override vs defaults vs env), token profile resolution from `tokens.env`, and ID sanitization/path normalization. Use temp directories to write `tokens.env` so tests remain hermetic (no repo fixtures required). Prefix test names with `runtime_helpers_` for filtering.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Tests cover at least one REST and one GraphQL URL resolution path.
  - Tests verify token profile lookup and error path for missing profiles.
- **Validation**:
  - `cargo test -p api-testing-core --lib runtime_helpers_`

## Sprint 2: Runner decomposition
**Goal**: Split `suite/runner.rs` into cohesive modules while keeping `run_suite` as the public entry point.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core`
- Verify: `suite_runner_loopback` continues to pass unchanged.

### Task 2.1: Create runner context + progress modules
- **Location**:
  - `crates/api-testing-core/src/suite/runner/mod.rs`
  - `crates/api-testing-core/src/suite/runner/context.rs`
  - `crates/api-testing-core/src/suite/runner/progress.rs`
- **Description**: Convert `suite/runner.rs` into a module directory. Move `SuiteRunOptions`, `SuiteRunOutput`, progress handling, and case metadata normalization into dedicated modules; keep `run_suite` in `runner/mod.rs`.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Public API remains `api_testing_core::suite::runner::run_suite` and type names unchanged.
  - File-level organization reduces `runner/mod.rs` to orchestration logic.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.2: Extract REST case preparation + safety gating
- **Location**:
  - `crates/api-testing-core/src/suite/runner/rest.rs`
  - `crates/api-testing-core/src/suite/runner/mod.rs`
- **Description**: Move REST request loading, safety checks, and auth/token preparation into `runner/rest.rs`, returning a structured `RestCasePlan` (or equivalent) that `run_suite` can execute without changing outputs.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - REST request parsing and safety decisions live in `runner/rest.rs`.
  - `run_suite` still produces identical stdout/stderr files for REST cases in `suite_runner_loopback`.
- **Validation**:
  - `cargo test -p api-testing-core --test suite_runner_loopback`

### Task 2.3: Extract REST execution + command snippet formatting
- **Location**:
  - `crates/api-testing-core/src/suite/runner/rest.rs`
  - `crates/api-testing-core/src/suite/runner/mod.rs`
- **Description**: Move REST request execution, response assertion handling, and command snippet formatting into `runner/rest.rs` so `run_suite` only orchestrates results.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - REST stdout/stderr contents and command snippets are byte-for-byte identical for existing tests.
  - `run_suite` delegates REST execution to `runner/rest.rs`.
- **Validation**:
  - `cargo test -p api-testing-core --test suite_runner_loopback`

### Task 2.4: Extract GraphQL case preparation + auth resolution
- **Location**:
  - `crates/api-testing-core/src/suite/runner/graphql.rs`
  - `crates/api-testing-core/src/suite/runner/mod.rs`
- **Description**: Move operation loading and JWT resolution into `runner/graphql.rs`, returning a structured plan object that `run_suite` can execute without changing outputs.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - GraphQL preparation logic lives in `runner/graphql.rs`.
  - `run_suite` behavior remains unchanged for GraphQL cases.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.5: Extract GraphQL execution + command snippet formatting
- **Location**:
  - `crates/api-testing-core/src/suite/runner/graphql.rs`
  - `crates/api-testing-core/src/suite/runner/mod.rs`
- **Description**: Move GraphQL execution, assertions, and command snippet formatting into `runner/graphql.rs` so `run_suite` only aggregates results and reporting.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 6
- **Acceptance criteria**:
  - GraphQL stdout/stderr files and command snippets are byte-for-byte identical for existing tests.
  - Shared case result construction stays centralized and reused by REST/GraphQL.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 2.6: Strengthen runner output assertions
- **Location**:
  - `crates/api-testing-core/tests/suite_runner_loopback.rs`
- **Description**: Extend `suite_runner_loopback` assertions to explicitly validate command snippet text, stdout/stderr file contents, and output file paths so refactors cannot silently change outputs.
- **Dependencies**:
  - Task 2.5
- **Complexity**: 4
- **Acceptance criteria**:
  - Tests assert command snippet formatting for at least one REST and one GraphQL case.
  - Tests assert stdout/stderr content and file path shape for existing cases.
- **Validation**:
  - `cargo test -p api-testing-core --test suite_runner_loopback`

## Sprint 3: Cleanup decomposition and consistency
**Goal**: Make `suite/cleanup` maintainable by splitting REST/GraphQL cleanup flows and sharing template logic.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core`
- Verify: cleanup steps still run and log as before.

### Task 3.1: Introduce cleanup module structure + shared template helpers
- **Location**:
  - `crates/api-testing-core/src/suite/cleanup/mod.rs`
  - `crates/api-testing-core/src/suite/cleanup/context.rs`
  - `crates/api-testing-core/src/suite/cleanup/template.rs`
- **Description**: Replace `suite/cleanup.rs` with a module directory, move context + template rendering into dedicated files, and re-export a thin `run_case_cleanup` entry point.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - `run_case_cleanup` compiles and delegates to the new module structure with no behavior change.
  - Template rendering helpers live in `cleanup/template.rs`.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 3.2: Move REST cleanup flow into submodule
- **Location**:
  - `crates/api-testing-core/src/suite/cleanup/mod.rs`
  - `crates/api-testing-core/src/suite/cleanup/context.rs`
  - `crates/api-testing-core/src/suite/cleanup/rest.rs`
  - `crates/api-testing-core/src/suite/cleanup/template.rs`
- **Description**: Move REST cleanup logic into `cleanup/rest.rs`, keeping logging and file outputs unchanged.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - REST cleanup still emits the same stderr log lines and files as before.
  - Template rendering and token resolution behavior remain unchanged.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 3.3: Move GraphQL cleanup flow into submodule
- **Location**:
  - `crates/api-testing-core/src/suite/cleanup/mod.rs`
  - `crates/api-testing-core/src/suite/cleanup/context.rs`
  - `crates/api-testing-core/src/suite/cleanup/graphql.rs`
  - `crates/api-testing-core/src/suite/cleanup/template.rs`
- **Description**: Move GraphQL cleanup logic into `cleanup/graphql.rs`, preserving operation loading, JWT handling, and logging output.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - GraphQL cleanup still emits identical stderr log lines and files as before.
  - `run_case_cleanup` behavior remains unchanged.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 3.4: Normalize cleanup error reporting
- **Location**:
  - `crates/api-testing-core/src/suite/cleanup/mod.rs`
  - `crates/api-testing-core/src/suite/cleanup/context.rs`
  - `crates/api-testing-core/src/suite/cleanup/rest.rs`
  - `crates/api-testing-core/src/suite/cleanup/graphql.rs`
  - `crates/api-testing-core/src/suite/cleanup/template.rs`
- **Description**: Centralize cleanup logging helpers (append, formatting, error prefixes) so REST and GraphQL cleanup share the same reporting flow and remain consistent with prior output.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 5
- **Acceptance criteria**:
  - Error log lines match previous formatting for common failure cases.
  - Cleanup failures still leave stderr artifacts in the same paths.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 3.5: Extend cleanup coverage tests
- **Location**:
  - `crates/api-testing-core/tests/suite_runner_loopback.rs`
  - `crates/api-testing-core/tests/suite_cleanup_graphql.rs`
- **Description**: Extend existing loopback tests to assert cleanup side effects and add a dedicated GraphQL cleanup test that asserts stderr logging and cleanup request behavior. Add any required fixtures under test temp directories to keep tests hermetic.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests cover REST cleanup and GraphQL cleanup flows.
  - Cleanup tests assert both success and failure logging paths.
- **Validation**:
  - `cargo test -p api-testing-core --test suite_runner_loopback --test suite_cleanup_graphql`

## Testing Strategy
- Unit: `#[cfg(test)]` helper tests in `crates/api-testing-core/src/suite/runtime.rs` (or `runtime_tests.rs`).
- Integration: extend `suite_runner_loopback` and add GraphQL cleanup loopback tests.
- E2E/manual: mandatory full workspace run with `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh` before delivery.

## Risks & gotchas
- Subtle output or error message changes during extraction could break CLI parity tests.
- Refactors may introduce accidental path normalization changes (absolute vs repo-relative).
- Splitting modules can create cyclic dependencies; keep data types in `context` modules to avoid that.
- Converting files to module directories can cause re-export/API regressions; add compile checkpoints after each split.

## Rollback plan
- Keep refactor work in small, reviewable commits per sprint; if regressions appear, revert the sprint commit(s) to restore the pre-refactor behavior.
- Preserve old module boundaries in the first sprint so rollback is a clean file-level revert.
- For module directory conversions, revert file-level splits first to restore old module paths and public re-exports.
