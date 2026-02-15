# Plan: api-gql SERVICE_TOKEN support and shared env fallback

## Overview
Add SERVICE_TOKEN as a GraphQL env fallback (after ACCESS_TOKEN) and ensure api-gql history records the actual env source. To avoid duplicate logic, extract a shared env-fallback resolver in api-testing-core and reuse it in both REST and GraphQL auth paths. Update tests and documentation to reflect the new behavior while preserving existing flags, defaults, and history formats, and update any spec/fixture docs if present.

## Scope
- In scope: shared env-fallback helper, GraphQL auth fallback update, api-gql history labeling, tests, docs, and any related spec/fixture updates.
- Out of scope: new CLI flags, changes to JWT profile resolution, changes to REST token/profile semantics beyond internal refactor.

## Assumptions (if any)
1. GraphQL fallback should mirror REST ordering: `ACCESS_TOKEN` first, then `SERVICE_TOKEN` when no JWT profile is selected.
2. History labeling should expose the actual env variable name used (e.g., `token=SERVICE_TOKEN`).
3. Existing history line structure remains unchanged aside from the env source label.

## Sprint 1: Shared Fallback + Core Auth Updates
**Goal**: Implement a shared env-fallback resolver and wire it into REST + GraphQL auth flows.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core -p api-gql -p api-rest`
- Verify: GraphQL auth falls back to `ACCESS_TOKEN` then `SERVICE_TOKEN` and history labels are correct.

### Task 1.1: Add shared env-fallback resolver
- **Location**:
  - `crates/api-testing-core/src/auth_env.rs` (new)
  - `crates/api-testing-core/src/lib.rs`
- **Description**: Create a helper that accepts an ordered list of env keys and returns `(token, env_name)` using `trim_non_empty` semantics. Prefer order as provided, return `None` if no env is set.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Helper returns the first non-empty env in order and the env name string.
  - Helper is exposed via `api_testing_core::auth_env`.
- **Validation**:
  - Unit tests cover order and empty/whitespace cases.

### Task 1.2: Refactor REST env fallback to shared helper
- **Location**:
  - `crates/api-rest/src/commands/call.rs`
- **Description**: Use the shared helper to resolve env fallback tokens for REST (still `ACCESS_TOKEN` then `SERVICE_TOKEN`). Preserve existing auth source tracking and history labels.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - No behavior change beyond refactor; existing REST tests remain green.
- **Validation**:
  - `cargo test -p api-rest`

### Task 1.3: Update GraphQL auth to include SERVICE_TOKEN fallback
- **Location**:
  - `crates/api-testing-core/src/graphql/auth.rs`
- **Description**: Replace the GraphQL env fallback with the shared helper. Update `GraphqlAuthSourceUsed` to carry the env name (e.g., `EnvFallback { env_name }`). Ensure ordering is `ACCESS_TOKEN` then `SERVICE_TOKEN`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - GraphQL auth resolves `ACCESS_TOKEN` first, `SERVICE_TOKEN` second when no JWT profile is selected.
  - Source tracking includes the actual env name used.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 1.4: Update api-gql history labeling for env auth source
- **Location**:
  - `crates/api-gql/src/commands/call.rs`
- **Description**: Log the actual env name in history lines (e.g., `token=ACCESS_TOKEN` or `token=SERVICE_TOKEN`) based on the updated `GraphqlAuthSourceUsed`.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 3
- **Acceptance criteria**:
  - History output reflects the env source used when fallback tokens are used.
  - The history header line includes `token=SERVICE_TOKEN` when that env is used and preserves field order.
- **Validation**:
  - `cargo test -p api-gql`

## Sprint 2: Tests + Documentation
**Goal**: Add coverage for SERVICE_TOKEN fallback and update docs for GraphQL auth behavior.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: Full test suite and formatting/lint checks pass.

### Task 2.1: Add tests for GraphQL fallback + history labeling
- **Location**:
  - `crates/api-testing-core/src/graphql/auth.rs` (tests)
  - `crates/api-gql/tests/history.rs`
- **Description**: Add tests that set `SERVICE_TOKEN` and assert GraphQL auth resolution uses it when `ACCESS_TOKEN` is absent; validate history includes `token=SERVICE_TOKEN`.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Tests cover `ACCESS_TOKEN` precedence, `SERVICE_TOKEN` fallback, and both envs set.
  - Tests cover whitespace-only `SERVICE_TOKEN` being ignored.
  - api-gql history test confirms correct labeling.
- **Validation**:
  - `cargo test -p api-testing-core -p api-gql`

### Task 2.2: Update documentation and any spec/fixtures (if present)
- **Location**:
  - `crates/api-testing-core/README.md`
  - `crates/api-gql/README.md`
  - `docs/` and `crates/api-gql/docs/` (if applicable)
- **Description**: Document GraphQL fallback behavior (now `ACCESS_TOKEN` then `SERVICE_TOKEN`) and note history labeling semantics.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
- **Complexity**: 2
- **Acceptance criteria**:
  - Docs explicitly list GraphQL fallback order and env variable names.
  - If spec/fixture docs exist for auth/history, they are updated accordingly.
- **Validation**:
  - Manual review of README sections for accuracy.

### Task 2.3: Full validation sweep
- **Location**:
  - repo root
- **Description**: Run the repo-required checks to confirm formatting, clippy, tests, and zsh completions.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 2
- **Acceptance criteria**:
  - All commands pass without warnings or errors.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: `api-testing-core` auth resolver + GraphQL auth resolution paths.
- Integration: `api-gql` history output in tests.
- E2E/manual: Not required beyond existing CLI integration tests; rely on repo checks.

## Risks & gotchas
- Changing `GraphqlAuthSourceUsed` shape impacts history logging and any other callers; update all call sites.
- Ensure fallback order remains `ACCESS_TOKEN` then `SERVICE_TOKEN` to avoid surprising behavior.
- Avoid changing JWT profile selection precedence or CLI flags.
- History output may be consumed by external tooling; preserve ordering and add regression tests.

## Rollback plan
- Revert commits affecting `auth_env`, `graphql/auth`, and `api-gql` history logging.
- Restore prior `GraphqlAuthSourceUsed` enum and history labeling.
- Re-run `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh` to confirm rollback integrity.
