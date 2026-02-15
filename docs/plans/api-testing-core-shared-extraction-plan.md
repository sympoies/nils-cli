# Plan: API testing shared extraction to api-testing-core

## Overview
This plan consolidates duplicated helper logic across `api-gql`, `api-rest`, and `api-test` into
`api-testing-core` while preserving CLI behavior and messages. The work proceeds in small, validated
refactors: first shared utility primitives, then shared command helpers for history/endpoint/report
pipelines, and finally cleanup plus full test gates. The intent is to reduce drift and make future
changes in one place without altering outputs or defaults.

## Scope
- In scope: new shared modules in `crates/api-testing-core/src`, refactors in `crates/api-gql/src`,
  `crates/api-rest/src`, `crates/api-test/src`, and tests that lock behavior parity.
- Out of scope: new CLI flags, output format changes, HTTP client swaps, or changing default URLs.

## Assumptions (if any)
1. Output, warnings, and exit codes for `api-gql`, `api-rest`, and `api-test` must remain unchanged.
2. It is acceptable to add new public modules in `api-testing-core` and update dependent crates.
3. Existing tests are the parity baseline; new tests can be added where behavior is currently implicit.

## Sprint 1: Shared utility primitives
**Goal**: Move duplicated small helpers into `api-testing-core` and reuse them everywhere.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core`
- Verify: New util tests pass and no existing tests change behavior.

### Task 1.1: Inventory duplicated helpers and define the target API surface
- **Location**:
  - `crates/api-gql/src/util.rs`
  - `crates/api-rest/src/util.rs`
  - `crates/api-test/src/main.rs`
  - `crates/api-testing-core/src/graphql/auth.rs`
  - `crates/api-testing-core/src/suite/resolve.rs`
  - `crates/api-testing-core/src/suite/runner/mod.rs`
- **Description**: Catalogue the repeated helpers (names, signatures, warning formats, and call
  sites) and define the target `api-testing-core` API surface to preserve current message text and
  default behaviors.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - A written inventory exists that maps each duplicate helper to a single shared replacement.
  - The target signatures specify how warning prefixes and output destinations are preserved.
- **Validation**:
  - `rg -n "trim_non_empty|bool_from_env|parse_u64_default|shell_quote|slugify|list_available_suffixes" crates/api-* crates/api-testing-core`

### Task 1.2: Create core shared util module and tests
- **Location**:
  - `crates/api-testing-core/src/cli_util.rs`
  - `crates/api-testing-core/src/lib.rs`
  - `crates/api-testing-core/Cargo.toml`
  - `crates/api-testing-core/tests/cli_util.rs`
- **Description**: Add a public `cli_util` module with the duplicated helpers used across CLIs
  (e.g., `trim_non_empty`, `parse_u64_default`, `shell_quote`, `slugify`, `maybe_relpath`,
  `list_available_suffixes`, `find_repo_root`, and report/history timestamps). Provide a
  `bool_from_env` helper that accepts a warning sink (writer or `Vec<String>`) and a tool label so
  current warning messages remain unchanged.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `cli_util` exposes the shared helpers with behavior matching current implementations.
  - Unit tests cover edge cases for each helper (empty strings, invalid numbers, suffix parsing).
- **Validation**:
  - `cargo test -p api-testing-core --test cli_util`

### Task 1.3: Migrate api-gql util usage to core
- **Location**:
  - `crates/api-gql/src/util.rs`
  - `crates/api-gql/src/commands/call.rs`
  - `crates/api-gql/src/commands/history.rs`
  - `crates/api-gql/src/commands/report.rs`
  - `crates/api-gql/src/commands/report_from_cmd.rs`
  - `crates/api-gql/src/commands/schema.rs`
  - `crates/api-gql/src/main.rs`
  - `crates/api-gql/Cargo.toml`
- **Description**: Replace local util functions with `api_testing_core::cli_util` equivalents and
  delete or slim down the local `util.rs` module. Preserve warning prefixes and error strings by
  passing the same tool label (`api-gql`) into shared helpers. Adjust `Cargo.toml` only if new test
  helpers require additional dev-deps.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - No local duplicate helper implementations remain in `api-gql`.
  - Existing tests and snapshots still match output and warning text.
- **Validation**:
  - `cargo test -p api-gql`

### Task 1.4: Migrate api-rest util usage to core
- **Location**:
  - `crates/api-rest/src/util.rs`
  - `crates/api-rest/src/commands/call.rs`
  - `crates/api-rest/src/commands/history.rs`
  - `crates/api-rest/src/commands/report.rs`
  - `crates/api-rest/src/main.rs`
  - `crates/api-rest/Cargo.toml`
- **Description**: Replace local util functions with `api_testing_core::cli_util` equivalents and
  delete or slim down the local `util.rs` module. Preserve warning prefixes and error strings by
  passing the same tool label (`api-rest`) into shared helpers. Adjust `Cargo.toml` only if new test
  helpers require additional dev-deps.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - No local duplicate helper implementations remain in `api-rest`.
  - Existing tests and snapshots still match output and warning text.
- **Validation**:
  - `cargo test -p api-rest`

### Task 1.5: Replace internal core duplicates with cli_util
- **Location**:
  - `crates/api-testing-core/src/graphql/auth.rs`
  - `crates/api-testing-core/src/suite/resolve.rs`
  - `crates/api-testing-core/src/suite/runner/mod.rs`
  - `crates/api-testing-core/src/suite/runner/graphql.rs`
  - `crates/api-testing-core/src/suite/runner/rest.rs`
- **Description**: Remove local copies of `trim_non_empty`, `parse_u64_default`, `shell_quote`, and
  `list_available_suffixes` inside core modules by routing through `cli_util`. Keep behavior and
  error messages identical to current implementations.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - No duplicate helper implementations remain in core modules listed above.
  - All existing core tests pass unchanged.
- **Validation**:
  - `cargo test -p api-testing-core`

## Sprint 2: Shared command helpers (history + endpoint resolution)
**Goal**: Consolidate higher-level command logic shared between REST and GraphQL CLIs.
**Demo/Validation**:
- Command(s): `cargo test -p api-gql` and `cargo test -p api-rest`
- Verify: History and endpoint resolution behavior is unchanged.

### Task 2.1: Extract shared history command runner
- **Location**:
  - `crates/api-testing-core/src/history.rs`
  - `crates/api-testing-core/src/cli_history.rs`
  - `crates/api-testing-core/tests/cli_history.rs`
  - `crates/api-gql/src/commands/history.rs`
  - `crates/api-rest/src/commands/history.rs`
- **Description**: Add a `cli_history` helper in core that encapsulates the common flow: resolve
  setup dir, locate history file (with env override), read records, apply `--tail` and
  `--command-only`, and return exit code `3` on empty history. Parameterize the helper with
  REST/GQL-specific config (env var names and setup resolver) so messages remain stable.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - `api-gql` and `api-rest` history commands delegate to the shared helper.
  - Existing exit codes and output formats remain identical.
- **Validation**:
  - `cargo test -p api-testing-core --test cli_history`
  - `cargo test -p api-rest --test history`
  - `cargo test -p api-gql`

### Task 2.2: Extract shared endpoint resolution helper
- **Location**:
  - `crates/api-testing-core/src/cli_endpoint.rs`
  - `crates/api-testing-core/tests/cli_endpoint.rs`
  - `crates/api-gql/src/commands/call.rs`
  - `crates/api-rest/src/commands/call.rs`
  - `crates/api-testing-core/src/env_file.rs`
- **Description**: Introduce a shared endpoint resolver that handles `--url`, `--env`,
  `*_ENV_DEFAULT`, and default URL fallbacks using a config struct (prefixes, env var names, default
  URL, and missing endpoints.env error message).
- **Dependencies**:
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - REST and GQL `resolve_endpoint_for_call` logic is fully delegated to the core helper.
  - Error messages for unknown envs and missing `endpoints.env` match current behavior.
- **Validation**:
  - `cargo test -p api-testing-core --test cli_endpoint`
  - `cargo test -p api-gql --tests`
  - `cargo test -p api-rest --tests`

### Task 2.3: Extract shared `--list-envs` helper
- **Location**:
  - `crates/api-testing-core/src/cli_endpoint.rs`
  - `crates/api-gql/src/commands/call.rs`
  - `crates/api-rest/src/commands/call.rs`
- **Description**: Add a helper for listing available endpoint suffixes from env files and reuse it
  for `--list-envs` in both CLIs. Ensure ordering and dedup behavior remains identical.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `--list-envs` output ordering and contents match pre-refactor behavior for REST and GQL.
- **Validation**:
  - `cargo test -p api-gql --tests`
  - `cargo test -p api-rest --tests`

## Sprint 3: Report command consolidation + final cleanup
**Goal**: Reduce duplication in report metadata/IO paths and finish the consolidation.
**Demo/Validation**:
- Command(s): `cargo test -p api-gql`, `cargo test -p api-rest`, `cargo test -p api-test`
- Verify: Report output and exit codes match pre-refactor behavior.

### Task 3.1: Centralize report metadata building
- **Location**:
  - `crates/api-testing-core/src/report.rs`
  - `crates/api-testing-core/src/cli_report.rs`
  - `crates/api-testing-core/tests/cli_report.rs`
  - `crates/api-gql/src/commands/report.rs`
  - `crates/api-rest/src/commands/report.rs`
- **Description**: Add a `cli_report` helper that computes `project_root`, `out_path`,
  `report_date`, and `generated_at`. It should honor existing env overrides (`GQL_REPORT_DIR`,
  `REST_REPORT_DIR`) and keep default naming/stamping identical.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Both REST and GQL report commands use the shared metadata helper.
  - Default output paths and date stamps remain unchanged.
- **Validation**:
  - `cargo test -p api-testing-core --test cli_report`
  - `cargo test -p api-gql --tests`
  - `cargo test -p api-rest --tests`

### Task 3.2: Centralize endpoint note helper for reports
- **Location**:
  - `crates/api-testing-core/src/cli_report.rs`
  - `crates/api-gql/src/commands/report.rs`
  - `crates/api-rest/src/commands/report.rs`
- **Description**: Add a helper that renders the endpoint note string for reports based on
  `--url`, `--env`, or implicit defaults. Ensure wording and precedence match current behavior.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Endpoint notes in generated reports remain unchanged for identical inputs.
- **Validation**:
  - `cargo test -p api-gql --tests`
  - `cargo test -p api-rest --tests`

### Task 3.3: Extract shared response-source reader
- **Location**:
  - `crates/api-testing-core/src/cli_io.rs`
  - `crates/api-testing-core/tests/cli_io.rs`
  - `crates/api-gql/src/commands/report.rs`
  - `crates/api-rest/src/commands/report.rs`
- **Description**: Add a helper for reading response bytes from `--response` (file or `-`/stdin),
  returning consistent errors. Use it in both report commands to remove duplicated stdin/file logic.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Response reading behavior (stdin vs file path) matches previous logic and error strings.
- **Validation**:
  - `cargo test -p api-testing-core --test cli_io`
  - `cargo test -p api-gql --tests`
  - `cargo test -p api-rest --tests`

### Task 3.4: Update api-test to use shared bool/env helpers and finalize cleanup
- **Location**:
  - `crates/api-test/src/main.rs`
  - `crates/api-testing-core/src/cli_util.rs`
  - `crates/api-test/Cargo.toml`
- **Description**: Replace the local `bool_from_env` in `api-test` with the core helper and remove
  any remaining duplicated utility code across the three CLIs. Add or update tests if needed and
  adjust `Cargo.toml` only if new dev-deps are required.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 3
- **Acceptance criteria**:
  - `api-test` uses shared helpers with unchanged warning text.
  - No remaining duplicate util implementations exist in the three CLI crates.
- **Validation**:
  - `cargo test -p api-test`

### Task 3.5: Full workspace validation gate
- **Location**:
  - `DEVELOPMENT.md`
- **Description**: Run the repo-required lint + test gate to confirm workspace-level parity after the
  refactor and capture any regressions early.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
  - Task 1.5
  - Task 2.1
  - Task 2.2
  - Task 2.3
  - Task 3.1
  - Task 3.2
  - Task 3.3
  - Task 3.4
- **Complexity**: 3
- **Acceptance criteria**:
  - All required checks pass with no new warnings.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: Add `cli_util`, `cli_history`, `cli_endpoint`, `cli_report`, and `cli_io` tests for helper
  edge cases and parameterized behaviors.
- Integration: Keep existing `api-rest`/`api-gql` command tests; add targeted tests for report path
  defaults and `--list-envs` output.
- E2E/manual: Ensure `crates/api-test/tests/e2e.rs` still passes and run the repo gate script.

## Risks & gotchas
- Subtle changes to warning text or default URL selection can break parity; keep shared helpers
  strictly data-driven and test exact messages.
- Shared helpers that accept writers vs string warnings can drift; enforce a single formatting
  function and reuse it consistently.
- Moving utilities may change visibility and module boundaries; avoid introducing cycles between
  core and CLI crates.

## Rollback plan
- Revert to pre-refactor commits for each sprint if parity tests fail.
- Keep temporary adapter functions in CLI crates until shared helpers have coverage, then delete.
