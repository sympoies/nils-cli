# Plan: codex-cli service-consumable diag/auth JSON contracts

## Overview
This plan keeps existing parity-first human output behavior for `codex-cli` `diag` and `auth`, while
adding stable machine-consumable JSON contracts for frontend and service integrations. Default
human-readable output stays unchanged, and JSON remains explicit opt-in with documented schema and
compatibility guarantees. The most critical gap is `diag rate-limits --all`, which must return raw
per-account usage JSON instead of the current table-only render path. The implementation is staged
to minimize regressions: contract definition first, then diag/auth behavior, then provider and
rollout alignment, and finally publish-readiness verification for the publishable crate.

## Scope
- In scope: `codex-cli diag rate-limits` JSON contract for single/all/async flows, `codex-cli auth`
  JSON contract for `use|refresh|auto-refresh|current|sync`, docs/spec updates, completion/help
  updates, provider capability metadata updates, and regression/contract tests.
- Out of scope: changing upstream ChatGPT API payloads, changing token refresh semantics,
  introducing new auth providers, or removing existing text output modes.

## Assumptions (if any)
1. Existing text output remains the default for CLI users and shell scripts that do not request JSON.
2. Service consumers can require explicit JSON mode and tolerate non-zero exit codes while still
   reading structured output.
3. Returning raw usage payloads for multi-account mode is acceptable as long as secrets/tokens are
   never emitted.
4. `nils-codex-cli` remains publishable and must continue to satisfy workspace release checks.

## Success Criteria
1. Existing human-readable `diag`/`auth` output and exit code semantics remain parity-safe by default.
2. `diag rate-limits --all --json` and `--async --json` emit stable, machine-consumable `results`
   payloads with non-sensitive `raw_usage`.
3. `auth` subcommands expose stable JSON contracts with structured error envelopes and service-ready
   metadata.
4. Required repository checks and publish-readiness validation pass for `nils-codex-cli`.

## Standards alignment (new CLI crate method)
- Follow `docs/runbooks/new-cli-crate-development-standard.md` as the workflow baseline:
  preserve human-readable defaults, add service JSON contracts explicitly, and keep publish-readiness
  gates in scope for publishable crates.
- Follow `docs/specs/cli-service-json-contract-guideline-v1.md` as the JSON contract baseline:
  required envelope keys (`schema_version`, `command`, `ok`), `result` for single-entity payloads,
  `results` for collections, and structured `error` envelope (`code`, `message`, optional `details`).
- Treat this plan’s crate-local contract doc (`crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`)
  as an extension of the generic guideline, not a divergent contract source.

## Contract direction (for review)
- JSON mode uses the required stable envelope keys `schema_version`, `command`, and `ok`, then
  `result` (single-account/single-target flows) or `results` (multi-account/list flows); optional
  metadata fields like `mode` and timestamps are additive.
- `diag rate-limits --all` JSON includes per-account `raw_usage` plus normalized summary fields used
  by current renderers.
- Auth commands expose outcome metadata (`matched_secret`, `target_file`, `refreshed`, `failed`,
  `reason`) so services do not parse prose.
- Usage errors continue to use existing exit code semantics; runtime JSON mode still prints
  structured output that callers can consume.

## Sprint 1: Contract and CLI surface alignment
**Goal**: Define the machine contract and wire parsing flags without changing behavior yet.
**Demo/Validation**:
- Command(s): `plan-tooling to-json --file docs/plans/codex-cli-service-consumable-diag-auth-plan.md --pretty`
- Verify: Tasks parse cleanly with dependencies/complexity and no placeholders.

### Task 1.1: Define versioned JSON contracts for diag/auth
- **Location**:
  - `crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `crates/codex-cli/README.md`
- **Description**: Define request/response envelopes and field-level semantics for `diag
  rate-limits` and `auth` commands, including success/failure payloads, exit-code mapping, and
  backward-compatibility rules for text mode, while explicitly inheriting
  `docs/specs/cli-service-json-contract-guideline-v1.md`.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Contract document includes concrete JSON examples for `diag` single/all/async and all auth
    subcommands.
  - Contract examples use `result` for single-entity responses and `results` for collection responses.
  - Contract defines which fields are stable for service parsing and which fields are informational.
  - Contract includes compatibility rules (additive changes only within v1; breaking key changes
    require a new schema version).
  - README links to the contract and states text-mode compatibility expectations.
- **Validation**:
  - `rg -n "\"schema_version\"|\"command\"|\"ok\"|diag rate-limits|auth use|auth refresh" crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `rg -n "cli-service-json-contract-guideline-v1|result|results|additive|breaking" crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `rg -n "stable fields|informational fields" crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `rg -n "codex-cli-diag-auth-json-contract-v1" crates/codex-cli/README.md`

### Task 1.2: Add output format flags across diag/auth CLI surfaces
- **Location**:
  - `crates/codex-cli/src/cli.rs`
  - `crates/codex-cli/src/main.rs`
- **Description**: Extend CLI parsing so `diag rate-limits` and `auth` commands accept structured
  output mode (JSON), preserving current flags and defaults. Keep `--json` compatibility for
  `diag`, and add a consistent format flag path for auth commands.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 7
- **Acceptance criteria**:
  - `diag rate-limits --json` remains accepted and maps to JSON mode.
  - `--format json` (or equivalent explicit format path) is supported for both `diag rate-limits`
    and `auth` surfaces, with `--json` kept for backward compatibility on `diag`.
  - Auth subcommands support JSON mode without changing default text output.
  - Help text clearly documents mode interactions and unsupported combinations.
- **Validation**:
  - `cargo run -p nils-codex-cli -- diag rate-limits --help`
  - `cargo run -p nils-codex-cli -- auth current --help`

### Task 1.3: Introduce shared JSON envelope models
- **Location**:
  - `crates/codex-cli/src/diag_output.rs`
  - `crates/codex-cli/src/auth/output.rs`
  - `crates/codex-cli/src/lib.rs`
- **Description**: Add typed structs and serializers for stable JSON envelopes used by both diag
  and auth code paths, including timestamp and command metadata fields.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - New output models are reused by diag/auth modules instead of ad-hoc JSON assembly.
  - Envelope always emits `schema_version`, `command`, and `ok`.
  - Serialization tests cover missing/partial data cases.
- **Validation**:
  - `cargo test -p nils-codex-cli --lib`

### Task 1.4: Add completion/help coverage for new output flags
- **Location**:
  - `completions/zsh/_codex-cli`
  - `completions/bash/codex-cli`
  - `crates/codex-cli/tests/main_entrypoint.rs`
- **Description**: Update shell completions and CLI help regression tests so JSON mode options are
  discoverable and stable for interactive and scripted usage.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Zsh and Bash completion list JSON mode flags for diag/auth where supported.
  - Help snapshots/tests cover the new options and do not regress existing alias behavior.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `cargo test -p nils-codex-cli main_entrypoint`

## Sprint 2: Diag multi-account raw JSON and parity-safe mode behavior
**Goal**: Make `diag rate-limits` fully machine-consumable for single/all/async flows.
**Demo/Validation**:
- Command(s): `cargo test -p nils-codex-cli rate_limits_`
- Verify: `--all` and `--async` JSON output are structured, deterministic, and include raw payloads.

### Task 2.1: Refactor rate-limit collectors to return typed per-account records
- **Location**:
  - `crates/codex-cli/src/rate_limits/mod.rs`
  - `crates/codex-cli/src/rate_limits/render.rs`
- **Description**: Extract shared collection logic from table render paths into typed records that
  capture account identity, normalized percentages, reset epochs, runtime status, and raw usage
  payload when available.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Sequential `--all` and `--async` flows share a common data model for rendering and JSON output.
  - Per-account record includes success/failure status and non-sensitive diagnostics.
  - Per-account `raw_usage` never includes local secret material (for example `access_token` and
    `refresh_token` keys).
  - Existing table output remains unchanged when JSON mode is not requested.
- **Validation**:
  - `cargo test -p nils-codex-cli rate_limits_network`
  - `cargo test -p nils-codex-cli rate_limits_raw_json`

### Task 2.2: Implement `diag rate-limits --all --json` with raw usage payloads
- **Location**:
  - `crates/codex-cli/src/rate_limits/mod.rs`
  - `crates/codex-cli/tests/rate_limits_all.rs`
  - `crates/codex-cli/tests/rate_limits_network.rs`
- **Description**: Replace the current `--all` JSON conflict with a structured JSON response that
  returns one object per account, including `raw_usage` payload, normalized summary fields, and
  explicit per-account error objects.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - `diag rate-limits --all --json` exits with the same success/failure semantics as text all-mode.
  - JSON payload includes both raw and normalized fields for each account.
  - Existing test that expects `--json is not supported with --all` is replaced by JSON contract tests.
- **Validation**:
  - `cargo test -p nils-codex-cli rate_limits_all`
  - `cargo test -p nils-codex-cli rate_limits_network -- --nocapture`

### Task 2.3: Implement `diag rate-limits --async --json` with the same schema
- **Location**:
  - `crates/codex-cli/src/rate_limits/mod.rs`
  - `crates/codex-cli/tests/rate_limits_async.rs`
- **Description**: Allow async mode to emit the same JSON schema as sequential all-mode, including
  deterministic ordering and per-account fallback metadata when cache is used.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 8
- **Acceptance criteria**:
  - `--async --json` is accepted and returns schema-compatible output with sequential mode.
  - Output ordering is deterministic regardless of worker completion order.
  - Async debug stderr behavior remains unchanged in text mode.
- **Validation**:
  - `cargo test -p nils-codex-cli rate_limits_async`

### Task 2.4: Add contract tests and negative-case tests for diag JSON consumers
- **Location**:
  - `crates/codex-cli/tests/rate_limits_single.rs`
  - `crates/codex-cli/tests/rate_limits_all.rs`
  - `crates/codex-cli/tests/rate_limits_async.rs`
  - `crates/codex-cli/tests/json.rs`
- **Description**: Add parsing assertions for required JSON fields and edge cases (missing secret
  dir, missing access token, mixed success/failure across accounts, cached fallback).
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests validate schema invariants instead of relying on prose output parsing.
  - Single-entity flows validate `result`; collection flows validate `results`.
  - Mixed-result scenarios still produce machine-readable `results` arrays.
  - Single-account JSON behavior remains backward-compatible.
- **Validation**:
  - `cargo test -p nils-codex-cli rate_limits_single`
  - `cargo test -p nils-codex-cli rate_limits_all`
  - `cargo test -p nils-codex-cli rate_limits_async`

## Sprint 3: Auth command JSON contracts for service integrations
**Goal**: Make auth subcommands parseable by services without scraping stdout text.
**Demo/Validation**:
- Command(s): `cargo test -p nils-codex-cli auth_`
- Verify: all auth subcommands emit stable JSON when requested and keep current text output by default.

### Task 3.1: Add JSON mode for `auth current`, `auth use`, and `auth sync`
- **Location**:
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/src/auth/current.rs`
  - `crates/codex-cli/src/auth/use_secret.rs`
  - `crates/codex-cli/src/auth/sync.rs`
  - `crates/codex-cli/tests/auth_current_sync.rs`
  - `crates/codex-cli/tests/auth_use.rs`
- **Description**: Implement structured JSON results for state/query and file-application auth
  commands, including matched secret, sync counts, and mismatch reasons.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - JSON output includes deterministic keys for match state and affected files.
  - Exit code semantics are unchanged for success, not-found, ambiguity, and usage errors.
  - Text output snapshots remain stable when JSON mode is not enabled.
- **Validation**:
  - `cargo test -p nils-codex-cli auth_current_sync`
  - `cargo test -p nils-codex-cli auth_use`

### Task 3.2: Add JSON mode for `auth refresh` and `auth auto-refresh`
- **Location**:
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/src/auth/refresh.rs`
  - `crates/codex-cli/src/auth/auto_refresh.rs`
  - `crates/codex-cli/tests/auth_refresh.rs`
  - `crates/codex-cli/tests/auth_auto_refresh.rs`
- **Description**: Emit structured JSON summaries for refresh operations, including refreshed
  targets, skipped targets, failures, and timestamp updates, while preserving current side effects.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - JSON includes counts and per-target status details for refresh workflows.
  - Failure paths emit structured error details without leaking token contents.
  - Existing refresh writeback behavior and timestamp updates remain unchanged.
- **Validation**:
  - `cargo test -p nils-codex-cli auth_refresh`
  - `cargo test -p nils-codex-cli auth_auto_refresh`

### Task 3.3: Standardize runtime error envelopes for JSON mode
- **Location**:
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/src/diag_output.rs`
  - `crates/codex-cli/src/auth/output.rs`
  - `crates/codex-cli/tests/main_entrypoint.rs`
- **Description**: Ensure runtime failures in JSON mode emit structured error objects with stable
  fields (`code`, `message`, optional `details`) across auth and diag commands.
- **Dependencies**:
  - Task 2.2
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Runtime JSON failures do not require stderr prose parsing for root-cause detection.
  - Usage-level clap errors continue to exit with existing codes and help behavior.
  - Error envelope fields are documented in the v1 contract doc.
- **Validation**:
  - `rg -n "\"code\"|\"message\"|\"details\"" crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `cargo test -p nils-codex-cli main_entrypoint`
  - `cargo test -p nils-codex-cli auth_`
  - `cargo test -p nils-codex-cli rate_limits_`

### Task 3.4: Add service-facing auth/diag JSON contract integration tests
- **Location**:
  - `crates/codex-cli/tests/auth_json_contract.rs`
  - `crates/codex-cli/tests/diag_json_contract.rs`
- **Description**: Add focused integration tests that treat `codex-cli` as a black box and verify
  machine-facing schema stability for representative success/failure scenarios.
- **Dependencies**:
  - Task 2.4
  - Task 3.1
  - Task 3.2
  - Task 3.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Contract tests validate required keys and basic type invariants.
  - Tests cover at least one partial-failure multi-account diag case and one mixed-result
    auto-refresh case.
  - Tests fail when a required field is removed or renamed.
- **Validation**:
  - `cargo test -p nils-codex-cli auth_json_contract`
  - `cargo test -p nils-codex-cli diag_json_contract`

## Sprint 4: Provider exposure, publish-readiness, rollout, and hardening
**Goal**: Make JSON contracts discoverable and safe for downstream service adoption while keeping
`nils-codex-cli` publish-ready.
**Demo/Validation**:
- Command(s): `cargo test -p agent-provider-codex`, `cargo test -p agentctl diag_capabilities`, and
  `scripts/publish-crates.sh --dry-run --crate nils-codex-cli`
- Verify: provider metadata exposes machine-consumable paths and publish dry-run remains green.

### Task 4.1: Update provider capability metadata for machine-consumable diag/auth
- **Location**:
  - `crates/agent-provider-codex/src/adapter.rs`
  - `crates/agent-provider-codex/tests/adapter_contract.rs`
  - `crates/agentctl/tests/diag_capabilities.rs`
- **Description**: Extend codex provider capability descriptions so downstream orchestrators can
  detect JSON-ready diag/auth operations and their expected stability level.
- **Dependencies**:
  - Task 2.2
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Capabilities surface clearly identifies JSON-consumable diagnostic/auth paths.
  - `agentctl diag capabilities --format json` includes updated capability metadata.
  - Existing non-experimental capability behavior remains unchanged.
- **Validation**:
  - `cargo test -p agent-provider-codex`
  - `cargo test -p agentctl diag_capabilities`

### Task 4.2: Publish migration guide for frontend/service consumers
- **Location**:
  - `crates/codex-cli/docs/runbooks/json-consumers.md`
  - `crates/codex-cli/README.md`
- **Description**: Document how services should call JSON mode, interpret exit codes, and handle
  partial failures. Include examples for multi-account rate limits and auth refresh pipelines.
- **Dependencies**:
  - Task 3.4
  - Task 4.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Guide includes command examples and response snippets for the main integration flows.
  - Guide defines do/don't practices for retrying and fallback handling.
  - README links to the new runbook.
- **Validation**:
  - `rg -n "json-consumers|diag rate-limits --all --json|auth auto-refresh" crates/codex-cli/docs/runbooks/json-consumers.md crates/codex-cli/README.md`

### Task 4.3: Run full required gate and targeted contract checks
- **Location**:
  - `DEVELOPMENT.md`
  - `crates/codex-cli/tests/diag_json_contract.rs`
  - `crates/codex-cli/tests/auth_json_contract.rs`
- **Description**: Execute repository-required quality gates plus targeted JSON contract tests to
  ensure readiness for downstream service integration.
- **Dependencies**:
  - Task 2.4
  - Task 3.4
  - Task 4.1
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Required lint/test gate passes.
  - JSON contract tests pass in CI-friendly environments.
  - No regressions in existing human-readable auth/diag tests.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `cargo test -p nils-codex-cli auth_json_contract diag_json_contract`

### Task 4.4: Verify publish-readiness gates for `nils-codex-cli`
- **Location**:
  - `crates/codex-cli/Cargo.toml`
  - `release/crates-io-publish-order.txt`
  - `scripts/publish-crates.sh`
- **Description**: Validate that contract/CLI surface changes keep `nils-codex-cli` compliant with
  publishable-crate requirements: metadata consistency, publish-order inclusion, and dry-run
  publishability.
- **Dependencies**:
  - Task 4.3
- **Complexity**: 3
- **Acceptance criteria**:
  - `crates/codex-cli/Cargo.toml` continues to follow workspace metadata conventions for a
    publishable crate.
  - `release/crates-io-publish-order.txt` contains `nils-codex-cli` in the expected publish list.
  - `scripts/publish-crates.sh --dry-run --crate nils-codex-cli` succeeds.
- **Validation**:
  - `rg -n "name = \"nils-codex-cli\"|version =|edition\\.workspace|license\\.workspace|repository =" crates/codex-cli/Cargo.toml`
  - `rg -n "^nils-codex-cli$" release/crates-io-publish-order.txt`
  - `scripts/publish-crates.sh --dry-run --crate nils-codex-cli`

## Dependency and parallelization plan
- Batch A (after Task 1.3): Task 2.1 and Task 3.1 can run in parallel because they touch mostly
  separate modules.
- Batch B (after Task 2.1 and Task 3.1): Task 2.2 and Task 3.2 can run in parallel with low file
  overlap.
- Batch C: Task 2.3 depends on Task 2.2 and should run after Batch B starts stabilizing.
- Batch D: Task 2.4 and Task 3.3 can run in parallel once core behaviors are merged.
- Batch E: Task 3.4 and Task 4.1 can run in parallel after Task 3.3.
- Batch F: Task 4.2 then Task 4.3 as integration hardening.
- Batch G: Task 4.4 as final publish-readiness gate.

## Testing Strategy
- Unit: serializer/envelope tests for diag/auth output models and error envelope helpers.
- Integration: command-level tests for `rate_limits_*`, `auth_*`, and new JSON contract suites.
- E2E/manual: smoke `codex-cli diag rate-limits --all --json` against loopback fixtures and verify
  `agentctl diag capabilities --format json` reflects new metadata.
- Release safety: run publish dry-run for `nils-codex-cli` after all contract/test gates pass.

## Risks & gotchas
- Contract drift risk: ad-hoc field additions can break consumers; keep schema versioned and test
  required keys.
- Sensitive data risk: raw payload forwarding must not include secrets/tokens from local files.
- Compatibility risk: auth command argument parsing currently uses positional vectors; migrating to
  richer flags must preserve legacy usage errors and exit codes.
- Async determinism risk: concurrent collection can reorder results unless explicit sorting is
  enforced before JSON emission.
- Publish risk: dependency or metadata drift can break crates.io dry-run even if tests pass.

## Rollback plan
- Keep default text behavior unchanged throughout implementation so rollback can disable JSON-only
  code paths without affecting existing users.
- If multi-account JSON causes regressions, temporarily reintroduce the `--all --json` guard while
  preserving single-account JSON and auth JSON improvements.
- Revert provider capability metadata changes independently if downstream tooling is not ready.
- If publish dry-run fails, pause release and roll back only metadata/order changes while keeping
  non-breaking runtime JSON additions behind current schema constraints.
- Use sprint-scoped revert points so each batch can be rolled back without undoing unrelated
  completed work.
