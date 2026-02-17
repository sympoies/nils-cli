# Plan: Consolidate Shared Test Helpers into nils-test-support

## Overview
Scan all crates for duplicated test helpers (CLI binary resolution, command runners, git repo fixtures, filesystem writers, HTTP test servers) and consolidate them into `crates/nils-test-support`. Add well-scoped modules with tests, then migrate crate tests to use the shared helpers. Keep production code untouched and preserve existing test behavior.

## Scope
- In scope: Rust test helpers in `crates/*/tests` and test modules under `crates/*/src`; new helper modules in `crates/nils-test-support`; refactors of tests to consume shared helpers.
- Out of scope: Production code changes, CLI behavior changes, large redesigns of API testing fixtures beyond what is needed for shared helpers.

## Assumptions (if any)
1. Minor API additions in `nils-test-support` are acceptable within the workspace (no external semver constraints).
2. Some crate-specific helpers will remain local if they are truly unique (only consolidate shared patterns).
3. Test behavior must remain identical (same env handling, outputs, and exit codes).

## Current duplication candidates (initial scan)
- Binary path helpers: `*_bin()` in `crates/api-gql/tests/*`, `crates/api-rest/tests/*`, `crates/api-test/tests/*`, `crates/codex-cli/tests/*`, `crates/cli-template/tests/cli.rs`, `crates/fzf-cli/tests/common.rs`, `crates/git-lock/tests/common.rs`, `crates/git-scope/tests/common.rs`, `crates/git-summary/tests/common.rs`, `crates/image-processing/tests/common.rs`, `crates/plan-tooling/tests/common.rs`, `crates/semantic-commit/tests/common.rs`.
- Command runners + `CmdOutput` structs: `crates/api-gql/tests/integration.rs`, `crates/api-rest/tests/endpoint_resolution.rs`, `crates/api-test/tests/e2e.rs`, `crates/plan-tooling/tests/common.rs`, `crates/image-processing/tests/common.rs`, `crates/fzf-cli/tests/common.rs`.
- Git repo helpers: `crates/git-lock/tests/common.rs`, `crates/git-scope/tests/common.rs`, `crates/git-summary/tests/common.rs`, `crates/plan-tooling/tests/common.rs`, `crates/semantic-commit/tests/common.rs`.
- Filesystem writers: `write_file/write_text/write_json/write_str` in `crates/api-gql/tests/integration.rs`, `crates/api-rest/tests/endpoint_resolution.rs`, `crates/api-test/tests/e2e.rs`, `crates/plan-tooling/tests/common.rs`, `crates/semantic-commit/tests/common.rs`.
- HTTP test servers and request parsing: `crates/api-gql/tests/integration.rs`, `crates/api-rest/tests/endpoint_resolution.rs`, `crates/api-test/tests/e2e.rs`.

## Sprint 1: Inventory + API design
**Goal**: Produce a consolidated inventory and design target helper APIs in `nils-test-support`.
**Demo/Validation**:
- Command(s): `rg -n "_bin\\(|CmdOutput|TempDir|read_until_headers_end|init_repo\\(" crates -g '*.rs'`
- Verify: Inventory notes include each duplicated helper and proposed destination module.

### Task 1.1: Create a consolidation inventory
- **Location**:
  - `crates/api-gql/tests`
  - `crates/api-rest/tests`
  - `crates/api-test/tests`
  - `crates/api-testing-core/tests`
  - `crates/cli-template/tests`
  - `crates/codex-cli/tests`
  - `crates/fzf-cli/tests`
  - `crates/git-lock/tests`
  - `crates/git-scope/tests`
  - `crates/git-summary/tests`
  - `crates/image-processing/tests`
  - `crates/nils-term/tests`
  - `crates/plan-tooling/tests`
  - `crates/semantic-commit/tests`
  - `crates/api-testing-core/src` (test modules)
  - `docs/plans/nils-test-support-consolidation-inventory.md`
- **Description**: Scan all crates for duplicated test helpers and record them in a dedicated inventory doc. Include file paths, helper names, and notes on uniqueness vs shared behavior.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Inventory doc lists each duplicated helper category with file locations.
  - Each item is tagged as "migrate" or "keep local" with rationale.
  - `#[cfg(test)]` helpers in `crates/*/src` are included or explicitly marked "keep local".
- **Validation**:
  - Review `docs/plans/nils-test-support-consolidation-inventory.md` for completeness against `rg` results.

### Task 1.2: Define target helper modules and APIs
- **Location**:
  - `crates/nils-test-support/src/lib.rs`
  - `crates/nils-test-support/src`
  - `docs/plans/nils-test-support-consolidation-inventory.md`
- **Description**: Design the new shared helper modules and function signatures (e.g., `bin::resolve`, `cmd::run`, `git::TempRepo`, `fs::write_text`, `http::TestServer`). Map each inventory item to a target API, document "keep local" criteria, and note any behavior constraints (env stripping, stdin support, headers capture, compatibility shims).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Inventory doc includes a mapping table from old helpers to new modules.
  - API signatures cover all observed use cases without expanding scope.
  - Explicit criteria exist for "keep local" helpers (uniqueness, coupling, or behavior that should not be generalized).
- **Validation**:
  - Manual review of mapping table vs inventory items.

### Task 1.3: Migration plan + sequencing
- **Location**:
  - `docs/plans/nils-test-support-consolidation-plan.md`
- **Description**: Finalize migration order and crate grouping to minimize conflicts (git-related crates together, API test crates together, CLI crates together). Identify parallelizable groups and dependencies.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Migration order documented in Sprint 3 tasks with explicit dependencies.
  - Inventory is the single source of truth for which crates are migrated vs kept local.
- **Validation**:
  - Spot-check that each crate with helpers appears in Sprint 3 tasks.

## Sprint 2: Implement shared helpers in nils-test-support
**Goal**: Add shared helper modules with tests and stable APIs.
**Demo/Validation**:
- Command(s): `cargo test -p nils-test-support`
- Verify: New helpers are covered by unit tests and compile without warnings.

### Task 2.1: Add binary path resolver + command runner
- **Location**:
  - `crates/nils-test-support/src/bin.rs`
  - `crates/nils-test-support/src/cmd.rs`
  - `crates/nils-test-support/src/lib.rs`
  - `crates/nils-test-support/tests`
- **Description**: Implement a reusable binary resolver that supports hyphen/underscore env var names (`CARGO_BIN_EXE_*`) and a command runner that captures `code/stdout/stderr` with optional stdin/env overrides.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - New helpers replace at least the patterns used in CLI tests (bin + CmdOutput).
  - Public API docs explain env handling and stdin behavior.
- **Validation**:
  - `cargo test -p nils-test-support --tests`

### Task 2.2: Add git repo fixture helpers
- **Location**:
  - `crates/nils-test-support/src/git.rs`
  - `crates/nils-test-support/src/lib.rs`
  - `crates/nils-test-support/tests`
- **Description**: Provide helpers for `git()` command execution, temp repo initialization (with deterministic branch), optional initial commit, and convenience methods like `commit_file`.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - APIs cover existing behaviors from git-lock/git-scope/git-summary/semantic-commit/plan-tooling tests.
- **Validation**:
  - `cargo test -p nils-test-support --tests`

### Task 2.3: Add filesystem writer helpers
- **Location**:
  - `crates/nils-test-support/src/fs.rs`
  - `crates/nils-test-support/src/lib.rs`
  - `crates/nils-test-support/tests`
- **Description**: Add helpers for `write_text`, `write_json`, `write_bytes`, and directory creation to replace local helpers in API and plan-tooling tests.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Helpers match semantics of existing writers (create parent dirs, preserve bytes).
- **Validation**:
  - `cargo test -p nils-test-support --tests`

### Task 2.4: Extend HTTP test server helpers
- **Location**:
  - `crates/nils-test-support/src/http.rs`
  - `crates/nils-test-support/tests`
- **Description**: Extend `LoopbackServer` or add a new `TestServer` to support request header capture and configurable JSON responses so api-gql/api-test/api-rest tests can share logic. Preserve current `LoopbackServer` behavior or provide a shim so existing callers remain compatible.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - API supports routing by method/path and inspecting headers/body.
  - Backward compatibility maintained for existing `LoopbackServer` callers.
- **Validation**:
  - `cargo test -p nils-test-support --tests`

## Sprint 3: Migrate crates to shared helpers
**Goal**: Replace duplicated helpers with `nils-test-support` modules.
**Demo/Validation**:
- Command(s): `cargo test -p git-lock -p git-scope -p git-summary -p plan-tooling -p semantic-commit`
- Verify: Git-related tests pass using new helpers.

### Task 3.1: Update dev-dependencies for migrated crates
- **Location**:
  - `crates/git-lock/Cargo.toml`
  - `crates/git-scope/Cargo.toml`
  - `crates/git-summary/Cargo.toml`
  - `crates/plan-tooling/Cargo.toml`
  - `crates/semantic-commit/Cargo.toml`
  - `crates/codex-cli/Cargo.toml`
  - `crates/cli-template/Cargo.toml`
  - `crates/fzf-cli/Cargo.toml`
  - `crates/image-processing/Cargo.toml`
  - `crates/api-rest/Cargo.toml`
  - `crates/api-gql/Cargo.toml`
  - `crates/api-test/Cargo.toml`
  - `crates/api-testing-core/Cargo.toml`
  - `crates/nils-term/Cargo.toml`
- **Description**: Add `nils-test-support` to `dev-dependencies` where needed, ensuring each migrated crate can import shared helpers. Keep dependency versions consistent with workspace policies.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
  - Task 2.4
- **Complexity**: 4
- **Acceptance criteria**:
  - All crates targeted for migration compile with `nils-test-support` in `dev-dependencies`.
- **Validation**:
  - `cargo check -p nils-test-support`

### Task 3.2: Migrate git-related crates + plan-tooling + semantic-commit
- **Location**:
  - `crates/git-lock/tests/common.rs`
  - `crates/git-scope/tests/common.rs`
  - `crates/git-summary/tests/common.rs`
  - `crates/plan-tooling/tests/common.rs`
  - `crates/semantic-commit/tests/common.rs`
- **Description**: Replace local git helpers and bin resolvers with `nils-test-support::git` and `nils-test-support::bin/cmd`. Remove duplicate functions once tests compile.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - All affected tests compile without local git helpers.
  - Behavior remains identical (same branch, same config, same outputs).
- **Validation**:
  - `cargo test -p git-lock -p git-scope -p git-summary -p plan-tooling -p semantic-commit`

### Task 3.3: Migrate CLI tests (codex-cli, cli-template, fzf-cli, image-processing)
- **Location**:
  - `crates/codex-cli/tests`
  - `crates/cli-template/tests/cli.rs`
  - `crates/fzf-cli/tests/common.rs`
  - `crates/image-processing/tests/common.rs`
- **Description**: Replace local `*_bin` and `CmdOutput` helpers with shared bin/cmd helpers. Keep crate-specific stubs (fzf/image) routed through existing `nils-test-support::stubs`.
- **Dependencies**:
  - Task 2.1
  - Task 2.3
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - CLI tests compile using shared helpers and existing behavior is preserved.
- **Validation**:
  - `cargo test -p codex-cli -p cli-template -p fzf-cli -p image-processing`

### Task 3.4: Migrate API test crates (api-rest, api-gql, api-test)
- **Location**:
  - `crates/api-rest/tests`
  - `crates/api-gql/tests`
  - `crates/api-test/tests`
- **Description**: Replace local HTTP server/request parsing helpers with `nils-test-support::http` extensions and reuse shared filesystem writers/fixtures for setup directories and suite files.
- **Dependencies**:
  - Task 2.3
  - Task 2.4
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - API tests compile using shared helpers and continue to validate expected behaviors.
- **Validation**:
  - `cargo test -p api-rest -p api-gql -p api-test`

### Task 3.5: Review remaining crates (api-testing-core, nils-term)
- **Location**:
  - `crates/api-testing-core/tests`
  - `crates/api-testing-core/src`
  - `crates/nils-term/tests`
- **Description**: Migrate any shared helpers found in `api-testing-core` or `nils-term` to `nils-test-support`; otherwise mark them as "keep local" in the inventory with rationale.
- **Dependencies**:
  - Task 2.1
  - Task 2.3
  - Task 2.4
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Any duplicated helpers are migrated or explicitly documented as "keep local".
- **Validation**:
  - `cargo test -p api-testing-core -p nils-term`

### Task 3.6: Clean up leftover helper modules
- **Location**:
  - `crates/git-lock/tests/common.rs`
  - `crates/git-scope/tests/common.rs`
  - `crates/git-summary/tests/common.rs`
  - `crates/plan-tooling/tests/common.rs`
  - `crates/semantic-commit/tests/common.rs`
  - `crates/fzf-cli/tests/common.rs`
  - `crates/image-processing/tests/common.rs`
- **Description**: Remove or slim down `common.rs` files that become unnecessary after migrations, keeping only truly crate-specific helpers.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
  - Task 3.4
  - Task 3.5
- **Complexity**: 4
- **Acceptance criteria**:
  - No duplicate helpers remain in tests; only unique helpers stay local.
- **Validation**:
  - `rg -n "fn .*_bin\(|CmdOutput|init_repo\(" crates/*/tests -g '*.rs'`

## Sprint 4: Full validation + docs polish
**Goal**: Ensure full test suite coverage and document new helpers.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify: Format, clippy, and tests pass for the workspace.

### Task 4.1: Workspace verification
- **Location**:
  - Workspace root
- **Description**: Run required formatting/linting/tests and fix any fallout from helper consolidation.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 5
- **Acceptance criteria**:
  - `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `zsh -f tests/zsh/completion.test.zsh` all succeed.
- **Validation**:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --workspace`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.2: Coverage + helper docs
- **Location**:
  - `crates/nils-test-support/README.md` (if exists)
  - `docs/` (if needed)
- **Description**: Document new helper modules and run coverage gate to ensure no regressions.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Coverage report meets the >=80% line requirement.
  - New helper APIs are documented for future test usage.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Testing Strategy
- Unit: `nils-test-support` tests for new modules (bin/cmd/git/fs/http).
- Integration: Per-crate test suites for migrated crates (prefer `cargo test -p <crate>` to include `#[cfg(test)]` modules).
- E2E/manual: Run `nils-cli-verify-required-checks` and coverage gates before final merge.

## Risks & gotchas
- Extending `http::RecordedRequest` or server behavior can break existing tests; preserve current API or add new types.
- Binary path resolution must handle both hyphenated and underscored `CARGO_BIN_EXE_*` env vars.
- Command runner defaults (stdin/env removal) must match prior per-crate behavior to avoid flakiness.
- Git helpers must keep deterministic branch and config settings for consistent diffs.
- Missing dev-dependency updates can cause confusing compile failures during migration.
- Using `--tests` only may skip `#[cfg(test)]` unit tests in `src/` modules.

## Rollback plan
- Revert nils-test-support helper changes and reintroduce per-crate `common.rs` helpers from git history.
- Keep migrations small and per-crate to allow partial rollback if a specific crate regresses.
