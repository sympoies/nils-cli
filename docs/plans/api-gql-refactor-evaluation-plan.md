# Plan: api-gql refactor evaluation + execution summary (Decision B)

## Overview
This document consolidates the evaluation plan and the executed Sprint 3 implementation for `api-gql`. The work is complete and focused on a local module split plus targeted tests, with no behavior changes and no shared extraction into `api-testing-core` beyond a schema precedence fix required for parity. All required repo checks and coverage validation have been run.

## Scope
- In scope:
  - `crates/api-gql` module split (`cli.rs`, `commands/*`, `util.rs`) and wiring in `main.rs`.
  - Test additions in `crates/api-gql/tests/` for parity gaps.
  - `api-testing-core` GraphQL schema precedence fix (`schema.local.env` overrides `schema.env`).
- Out of scope:
  - New CLI flags or behavior changes.
  - Shared extraction into `api-testing-core` beyond the schema precedence fix.

## Decision
Decision: **B — local module split only**.
Rationale:
- `crates/api-gql/src/main.rs` was 1849 LOC with mixed responsibilities.
- Coverage baseline was 73.89% (< 80%).
- Shared extraction should wait for alignment with `docs/plans/api-testing-core-refactor-plan.md`.

## Implementation Summary
### Module layout (now in place)
- `crates/api-gql/src/main.rs`: entrypoint, argv defaulting, root help, dispatch only.
- `crates/api-gql/src/cli.rs`: Clap structs (`Cli`, `Command`, `*Args`).
- `crates/api-gql/src/commands/`:
  - `call.rs`: `cmd_call`, `cmd_call_internal`, endpoint resolution, history append.
  - `history.rs`: `cmd_history`.
  - `report.rs`: `cmd_report`, response gating, command snippet builder.
  - `report_from_cmd.rs`: `cmd_report_from_cmd`, dry-run command builder.
  - `schema.rs`: `cmd_schema`.
  - `mod.rs`: exports.
- `crates/api-gql/src/util.rs`: shared helpers + moved unit tests.

### Parity-related fix
- `schema.local.env` precedence now overrides `schema.env` in `api-testing-core` schema resolution.
  - File: `crates/api-testing-core/src/graphql/schema_file.rs`

## Test Additions (Parity Gaps Closed)
- Env/auth resolution:
  - `--list-jwts` output and missing jwts file error.
  - `GQL_ENV_DEFAULT` fallback.
  - `GQL_URL` env override.
  - `ACCESS_TOKEN` fallback when no JWT profile.
- History:
  - Exit code 3 for empty history file.
  - `--command-only` output formatting.
- Report:
  - `--run` path executes call and writes report.
  - `--response -` stdin path.
  - Non-JSON response refusal without allow-empty.
  - Redaction default vs `--no-redact`.
  - `--no-command` and `--no-command-url` toggles.
- Schema:
  - `schema.local.env` overrides `schema.env`.

## Validation Results
- Coverage (post-change): **85.87%** (1070/1246)
  - Command: `cargo llvm-cov nextest --profile ci -p api-gql --lcov --output-path target/coverage/api-gql.lcov.info`
  - Summary: `scripts/ci/coverage-summary.sh target/coverage/api-gql.lcov.info`
- Repo-required checks:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - Result: all checks passed (fmt, clippy, tests, zsh completion).

## Current Status
- All tasks in Sprint 3 are complete.
- No known remaining gaps for the parity behaviors listed in this scope.

## Files Touched (Summary)
- `crates/api-gql/src/main.rs`
- `crates/api-gql/src/cli.rs`
- `crates/api-gql/src/util.rs`
- `crates/api-gql/src/commands/call.rs`
- `crates/api-gql/src/commands/history.rs`
- `crates/api-gql/src/commands/report.rs`
- `crates/api-gql/src/commands/report_from_cmd.rs`
- `crates/api-gql/src/commands/schema.rs`
- `crates/api-gql/src/commands/mod.rs`
- `crates/api-gql/tests/env_and_auth_resolution.rs`
- `crates/api-gql/tests/history.rs`
- `crates/api-gql/tests/integration.rs`
- `crates/api-gql/tests/schema_command.rs`
- `crates/api-testing-core/src/graphql/schema_file.rs`

## Rollback Plan
- Revert to pre-refactor `main.rs` layout; keep only tests that pass on the legacy structure.

