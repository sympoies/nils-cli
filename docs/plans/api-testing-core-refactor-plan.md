# Plan: API testing core refactor for maintainability + coverage

## Overview
This plan refactors `crates/api-testing-core` to improve maintainability and raise test coverage while
preserving the external behavior of `api-rest`, `api-gql`, and `api-test`. The approach is to lock
current behavior with characterization tests, then restructure duplicated setup discovery, auth/token
resolution, and report/history pipelines into shared, testable components. The refactor favors small,
validated steps and keeps CLI-facing output stable.

## Scope
- In scope: `crates/api-testing-core` module refactors, new test fixtures and integration tests, and
  small adapter changes in `api-rest`, `api-gql`, and `api-test` to match refactored core APIs.
- Out of scope: New CLI flags, behavior changes, HTTP client swaps, or changes to legacy scripts.

## Assumptions (if any)
1. Output and exit-code parity for `api-rest`, `api-gql`, and `api-test` must remain unchanged.
2. Tests can use `nils-test-support` helpers and loopback HTTP servers where needed.
3. Internal API changes are allowed if dependent crates are updated in the same refactor.

## Sprint 1: Characterization + test scaffolding
**Goal**: Lock current behavior and build reusable fixtures to enable safe refactors.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core`
- Verify: New characterization tests pass and fail on intentional behavior changes.
**Parallelizable**: After Task 1.1, Tasks 1.2 and 1.3 can run in parallel.

### Task 1.1: Add fixture builders and test support helpers
- **Location**:
  - `crates/api-testing-core/tests/support/mod.rs`
  - `crates/api-testing-core/tests/fixtures_smoke.rs`
  - `crates/nils-test-support/src/http.rs`
- **Description**: Create reusable helpers to build temp setup dirs (endpoints/tokens/jwts env files),
  stub request/operation files, and spin up loopback HTTP servers. Add a small smoke test to ensure
  the helpers assemble a minimal repo layout correctly.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - A fixture helper can create a REST + GraphQL setup directory with endpoints and tokens.
  - A smoke test verifies the fixture helper output structure.
- **Validation**:
  - `cargo test -p api-testing-core --test fixtures_smoke`

### Task 1.2: Characterize setup discovery and config resolution
- **Location**:
  - `crates/api-testing-core/src/config.rs`
  - `crates/api-testing-core/tests/config_resolution.rs`
- **Description**: Add tests that capture the exact discovery order for REST/GQL setup dirs,
  including config-dir overrides, upward search rules, and invocation-dir fallbacks.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests cover at least one REST and one GQL discovery path per rule branch.
  - Each test asserts the resolved setup dir matches current behavior.
- **Validation**:
  - `cargo test -p api-testing-core --test config_resolution`

### Task 1.3: Characterize report, history, and snippet formatting
- **Location**:
  - `crates/api-testing-core/src/report.rs`
  - `crates/api-testing-core/src/markdown.rs`
  - `crates/api-testing-core/src/redact.rs`
  - `crates/api-testing-core/src/cmd_snippet.rs`
  - `crates/api-testing-core/src/history.rs`
  - `crates/api-testing-core/tests/report_history.rs`
- **Description**: Add tests for report markdown structure, redaction rules, history record
  formatting, and command snippet rendering. Focus on output invariants and edge cases like
  redacting secrets and preserving line breaks.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests assert stable markdown headings and command snippet formatting.
  - Tests cover history rotation and command-only extraction paths.
- **Validation**:
  - `cargo test -p api-testing-core --test report_history`

## Sprint 2: Unify setup discovery + auth resolution
**Goal**: Reduce duplicated discovery and token logic while keeping behavior identical.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core --test config_resolution`
- Verify: Updated code passes existing characterization tests without changes.
**Parallelizable**: none.

### Task 2.1: Extract shared setup discovery model
- **Location**:
  - `crates/api-testing-core/src/config.rs`
  - `crates/api-rest/src/main.rs`
  - `crates/api-gql/src/main.rs`
  - `crates/api-test/src/main.rs`
- **Description**: Introduce a `SetupDiscovery` model (for REST/GQL) that centralizes search rules
  and fallback ordering. Replace the per-command functions with a shared resolver while preserving
  exact error messages.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - REST and GQL discovery routes use the shared resolver.
  - `config_resolution` tests pass unchanged.
- **Validation**:
  - `cargo test -p api-testing-core --test config_resolution`

### Task 2.2: Centralize token/JWT profile resolution
- **Location**:
  - `crates/api-testing-core/src/suite/runtime.rs`
  - `crates/api-testing-core/src/graphql/auth.rs`
  - `crates/api-testing-core/src/rest/runner.rs`
  - `crates/api-testing-core/src/env_file.rs`
- **Description**: Move token/JWT profile resolution into a shared helper (new module or extended
  `env_file` API). Remove duplicated key normalization logic and ensure error messages match
  current behavior.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - REST token and GraphQL JWT lookups use the shared helper.
  - Existing auth/error tests (and new ones from Sprint 1) pass unchanged.
- **Validation**:
  - `cargo test -p api-testing-core --test report_history`
  - `cargo test -p api-testing-core --test suite_runner_loopback`

### Task 2.3: Introduce a resolved-setup struct for core workflows
- **Location**:
  - `crates/api-testing-core/src/config.rs`
  - `crates/api-testing-core/src/history.rs`
  - `crates/api-testing-core/src/report.rs`
  - `crates/api-testing-core/src/rest/runner.rs`
  - `crates/api-testing-core/src/graphql/runner.rs`
- **Description**: Add a `ResolvedSetup` struct that carries setup dir, history file, and env file
  paths. Update call sites to pass this struct instead of raw paths to reduce parameter sprawl.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Core entrypoints use `ResolvedSetup` and no longer duplicate path resolution.
  - All Sprint 1 tests pass without expectation changes.
- **Validation**:
  - `cargo test -p api-testing-core`

## Sprint 3: Report + history pipeline refactor
**Goal**: Consolidate report/history logic into testable, reusable pipelines.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core --test report_history`
- Verify: Output format tests remain stable after refactor.
**Parallelizable**: After Task 2.3, Tasks 3.1 and 3.2 can run in parallel.

### Task 3.1: Unify report rendering pipeline
- **Location**:
  - `crates/api-testing-core/src/report.rs`
  - `crates/api-testing-core/src/rest/report.rs`
  - `crates/api-testing-core/src/graphql/report.rs`
  - `crates/api-testing-core/src/markdown.rs`
  - `crates/api-testing-core/src/redact.rs`
- **Description**: Create a shared report builder that assembles markdown sections, applies
  redaction consistently, and renders command snippets. Update REST/GQL report modules to call the
  shared pipeline and keep the same output ordering.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - REST and GQL reports are rendered through a single shared pipeline.
  - Report formatting tests match the pre-refactor outputs exactly.
- **Validation**:
  - `cargo test -p api-testing-core --test report_history`

### Task 3.2: Extract a history writer with deterministic formatting
- **Location**:
  - `crates/api-testing-core/src/history.rs`
  - `crates/api-testing-core/src/rest/runner.rs`
  - `crates/api-testing-core/src/graphql/runner.rs`
- **Description**: Wrap history append + rotation logic in a small `HistoryWriter` struct with
  explicit formatting helpers. Keep lock behavior and rotation policies identical.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 5
- **Acceptance criteria**:
  - History append output is unchanged for identical inputs.
  - Rotation tests cover the new writer API.
- **Validation**:
  - `cargo test -p api-testing-core --test report_history`

## Sprint 4: Suite runtime refactor + coverage gates
**Goal**: Improve suite runtime maintainability and raise coverage for core workflows.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core`, `cargo llvm-cov nextest --profile ci -p api-testing-core --lcov --output-path target/coverage/api-testing-core.lcov.info`
- Verify: Coverage for `api-testing-core` is >= 80% and workspace gates still pass.
**Parallelizable**: After Task 4.1, Task 4.2 can run in parallel with Task 3.2 if not already done.

### Task 4.1: Split suite runtime planning from execution
- **Location**:
  - `crates/api-testing-core/src/suite/runtime.rs`
  - `crates/api-testing-core/src/suite/resolve.rs`
  - `crates/api-testing-core/src/suite/results.rs`
  - `crates/api-testing-core/src/suite/summary.rs`
- **Description**: Separate pure planning (path resolution, URL selection, token selection, output
  path computation) from side-effectful execution. Add unit tests for planning functions and result
  rendering.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Planning functions are pure and unit-tested with fixtures.
  - Suite result rendering tests cover success and failure cases.
- **Validation**:
  - `cargo test -p api-testing-core --test suite_planning`

### Task 4.2: Expand suite integration tests with loopback servers
- **Location**:
  - `crates/api-testing-core/tests/suite_runner_loopback.rs`
  - `crates/api-testing-core/tests/suite_cleanup_graphql.rs`
  - `crates/api-testing-core/tests/suite_rest_graphql_matrix.rs`
- **Description**: Add integration tests that run REST and GraphQL cases through the suite runner,
  asserting report output, redaction, and summary JSON. Use loopback servers to avoid external
  dependencies.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Integration tests cover REST-only, GQL-only, and mixed suites.
  - Summary JSON includes expected status counts and durations.
- **Validation**:
  - `cargo test -p api-testing-core --test suite_rest_graphql_matrix`

### Task 4.3: Coverage gate and workspace validation
- **Location**:
  - `DEVELOPMENT.md`
  - `scripts/ci/coverage-summary.sh`
- **Description**: Run the repo-required checks and capture coverage for the refactored crate.
  Ensure `api-testing-core` coverage meets the 80% target and workspace gates still pass.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `api-testing-core` line coverage >= 80%.
  - Workspace fmt/clippy/tests and zsh completion tests pass.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci -p api-testing-core --lcov --output-path target/coverage/api-testing-core.lcov.info --fail-under-lines 80`
  - `scripts/ci/coverage-summary.sh target/coverage/api-testing-core.lcov.info`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: Pure helpers (config discovery, env parsing, report markdown, suite planning) with fixture-based tests.
- Integration: Loopback HTTP servers for REST/GQL suite runs, plus history/report generation checks.
- E2E/manual: Validate `api-rest`, `api-gql`, and `api-test` commands against a sample repo layout.

## Risks & gotchas
- Behavior drift in report formatting or discovery order; mitigate with characterization tests first.
- Flaky tests due to time-dependent fields; mitigate with deterministic time injection or fixed clocks in tests.
- Over-refactor of shared helpers could leak changes into CLI outputs; keep tests as the gate before merges.

## Rollback plan
- Revert the refactor commits while retaining the new characterization tests to preserve behavior checks.
- Restore previous module APIs if dependent crates regress, then re-apply refactor in smaller steps.
