# Plan: Claude Code version for codex-cli capabilities

> Status: Superseded by `docs/plans/codex-claude-unified-cli-core-agentctl-plan.md` (2026-02-19).
>
> This document is retained for historical traceability only. Any scope constraints in this file
> that conflict with the unified dual-CLI direction are historical and must not be used for new
> implementation decisions.

## Overview
This plan delivers a Claude-backed equivalent for codex-oriented agent workflows without relying on
Claude internals being open source. The implementation target is the existing provider architecture:
`agent-provider-claude` + `agentctl`, with a codex-to-claude parity mapping so users can migrate
workflows safely. Correctness is defined by a multi-oracle strategy: official API contracts first,
black-box characterization second, and deterministic fixture tests as the release gate.

## Scope
- In scope:
  - Promote `agent-provider-claude` from `stub` to `stable` for `provider-adapter.v1`.
  - Deliver Claude execute/auth-state/healthcheck/capabilities/limits behavior with deterministic
    error mapping.
  - Define a codex-cli capability parity matrix (`exact` / `semantic` / `unsupported`) for Claude.
  - Add verification infrastructure that does not depend on Claude source code availability.
  - Update `agentctl` diagnostics/workflow tests and operator docs for Claude readiness.
- Out of scope:
  - Reverse-engineering or cloning proprietary Claude internal behavior.
  - Forcing 1:1 parity for Codex-only surfaces that have no Claude API equivalent.
  - (Historical under superseded scope) Changing `codex-cli` ownership boundaries or removing
    existing Codex paths.

## Assumptions (if any)
1. The authoritative runtime contract remains `provider-adapter.v1` in
   `crates/agent-runtime-core`.
2. “Codex-cli Claude version” means functional parity for user workflows, not byte-for-byte output
   parity across providers.
3. Official Anthropic API documentation and observed API responses are the primary source of truth;
   Claude CLI behavior is a secondary oracle.
4. Live API smoke tests are opt-in and not required for offline CI determinism.

## Success Criteria
1. `agent-provider-claude` reports `maturity=stable` and `execute.available=true` when configured.
2. `agentctl workflow run` can execute provider steps with `provider=claude` deterministically.
3. A documented parity matrix exists for each codex-cli capability, with explicit unsupported paths
   and stable error behavior.
4. Required checks pass, including repository required checks and coverage gate.

## Correctness oracles (closed-source-safe)
- Primary oracle: official API contract (request/response fields, error codes, rate-limit semantics).
- Secondary oracle: black-box characterization of Claude CLI (stdout/stderr/exit behavior) only for
  overlapping flows.
- Tertiary oracle: local deterministic fixtures and mock responses used by CI.
- Conflict rule: if oracles disagree, prefer official API + adapter contract, then document the
  divergence in parity matrix and tests.

## Parallelization notes
- After Task 1.2 lands, Task 1.3 (oracle runbook) and Task 1.4 (fixture manifest) can run in
  parallel.
- After Task 2.2 lands, Task 2.3 (execute pathway) and Task 2.4 (non-execute surfaces) can run in
  parallel.
- In Sprint 3, Task 3.2 (workflow integration tests) and Task 3.3 (migration mapping docs) can run
  in parallel.

## Sprint 1: Contract baseline and parity definition
**Goal**: Freeze what “correct” means before implementation, including how to validate a closed-source provider.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/codex-cli-claude-code-version-plan.md`
  - `plan-tooling to-json --file docs/plans/codex-cli-claude-code-version-plan.md --pretty >/dev/null`
- Verify: tasks parse, dependencies resolve, and required fields are complete.

### Task 1.1: Build codex-to-claude capability parity matrix
- **Location**:
  - `crates/agent-provider-claude/docs/specs/codex-cli-claude-parity-matrix-v1.md`
  - `crates/codex-cli/src/cli.rs`
  - `crates/codex-cli/README.md`
- **Description**: Inventory `codex-cli` command families (`agent`, `auth`, `diag`, `config`,
  `starship`) and classify each capability into `exact`, `semantic`, or `unsupported` for Claude.
  For unsupported capabilities, define stable fallback/error behavior and migration guidance.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Matrix lists every codex-cli command surface and classification.
  - Each non-exact mapping includes rationale and user-visible behavior.
  - Matrix identifies parity-critical behavior (exit codes, JSON envelope, warning style).
- **Validation**:
  - `rg -n "exact|semantic|unsupported|agent|auth|diag|config|starship" crates/agent-provider-claude/docs/specs/codex-cli-claude-parity-matrix-v1.md`

### Task 1.2: Define Claude adapter contract and error taxonomy
- **Location**:
  - `crates/agent-provider-claude/docs/specs/claude-provider-contract-v1.md`
  - `crates/agent-runtime-core/src/schema.rs`
  - `crates/agent-provider-claude/src/adapter.rs`
- **Description**: Write a versioned contract doc for Claude adapter behavior under
  `provider-adapter.v1`, including request shaping, output normalization, and category/code mapping
  for auth/network/timeout/rate-limit/validation errors.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Contract defines stable behavior for all five operations (`capabilities`, `healthcheck`,
    `execute`, `limits`, `auth-state`).
  - Error mapping table includes provider-adapter category, stable code, retryability, and
    redaction rules.
  - Contract includes explicit compatibility policy for additive vs breaking changes.
- **Validation**:
  - `rg -n "capabilities|healthcheck|execute|limits|auth-state|retryable|compatibility" crates/agent-provider-claude/docs/specs/claude-provider-contract-v1.md`

### Task 1.3: Publish verification-oracle runbook
- **Location**:
  - `crates/agent-provider-claude/docs/runbooks/verification-oracles.md`
  - `crates/agent-provider-claude/README.md`
- **Description**: Document the closed-source-safe validation workflow: which checks rely on API
  contracts, which rely on black-box characterization, how discrepancies are triaged, and what must
  pass before promoting release maturity.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Runbook defines oracle priority and dispute resolution.
  - Runbook defines minimum reproducible fixture set and pass/fail criteria.
  - Runbook defines mismatch severity and release policy:
    - API contract mismatch = release blocker.
    - Fixture-vs-adapter mismatch = release blocker.
    - CLI-only mismatch with API-consistent adapter = non-blocking but must be documented.
  - Characterization artifacts require `api_doc_date`, `model_id`, `claude_cli_version`, and
    `fixture_schema_version`.
  - README links to this runbook.
- **Validation**:
  - `rg -n "Primary oracle|Secondary oracle|Tertiary oracle|discrepancy|release gate" crates/agent-provider-claude/docs/runbooks/verification-oracles.md`
  - `rg -n "verification-oracles" crates/agent-provider-claude/README.md`

### Task 1.4: Create deterministic fixture manifest for characterization and mocks
- **Location**:
  - `crates/agent-provider-claude/tests/fixtures/README.md`
  - `crates/agent-provider-claude/tests/fixtures/characterization/manifest.json`
  - `crates/agent-provider-claude/tests/fixtures/api/rate_limit_error.json`
- **Description**: Define fixture IDs, prompt inputs, expected adapter envelopes, and masked API
  payloads for deterministic tests. Include explicit redaction policy and fixture update rules.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Fixture manifest covers success, auth failure, rate-limit, timeout, and malformed-response cases.
  - Fixture naming/versioning policy is documented.
  - No fixture contains secrets or unredacted tokens.
- **Validation**:
  - `test -f crates/agent-provider-claude/tests/fixtures/README.md`
  - `rg -n "\"id\"\\s*:\\s*\"(success|auth_failure|rate_limit|timeout|malformed_response)\"" crates/agent-provider-claude/tests/fixtures/characterization/manifest.json`
  - `rg -n "(?i)(api[_-]?key|authorization:|bearer [^\\\"]+|sk-[a-z0-9]{20,})" crates/agent-provider-claude/tests/fixtures && (echo "secret-like token found" && exit 1) || true`

## Sprint 2: Claude adapter implementation (stub -> stable)
**Goal**: Implement a production-ready Claude adapter with deterministic behavior under provider-adapter.v1.
**Demo/Validation**:
- Command(s): `cargo test -p nils-agent-provider-claude`
- Verify: adapter contract tests pass and execute/auth-state are implemented (non-stub behavior).

### Task 2.1: Add Claude config and auth resolution primitives
- **Location**:
  - `crates/agent-provider-claude/src/config.rs`
  - `crates/agent-provider-claude/src/lib.rs`
  - `crates/agent-provider-claude/tests/config_contract.rs`
  - `crates/agent-provider-claude/Cargo.toml`
- **Description**: Add configuration parsing for Claude execution (API key path/env, base URL,
  model defaults, timeout) and auth-state helpers with redaction-safe diagnostics.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Config resolver validates required fields and returns normalized errors.
  - Missing/invalid auth configuration maps to stable provider error categories/codes.
  - Public API exposes config/auth helpers used by adapter and tests.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test config_contract`

### Task 2.2: Implement Claude API client with deterministic retries/timeouts
- **Location**:
  - `crates/agent-provider-claude/src/client.rs`
  - `crates/agent-provider-claude/src/adapter.rs`
  - `crates/agent-provider-claude/tests/client_contract.rs`
  - `crates/agent-provider-claude/Cargo.toml`
- **Description**: Add a blocking HTTP client module for Claude API calls with timeout handling,
  retry policy for transient failures, and stable error mapping to provider-adapter categories.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Client handles success, 4xx, 5xx, timeout, and network errors deterministically.
  - Retry policy applies only to configured retryable categories.
  - Error details are useful for debugging without leaking credentials.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test client_contract`

### Task 2.3: Implement execute pathway and prompt templating parity
- **Location**:
  - `crates/agent-provider-claude/src/adapter.rs`
  - `crates/agent-provider-claude/src/prompts.rs`
  - `crates/agent-provider-claude/tests/execute_contract.rs`
- **Description**: Implement `execute` using Claude API client and map codex-style agent intents
  (`prompt`, `advice`, `knowledge`) into deterministic prompt templates and output envelopes.
- **Dependencies**:
  - Task 2.2
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Empty task/input validation matches provider-adapter requirements.
  - Success and failure envelopes are stable and covered by tests.
  - Prompt template mapping is documented and testable.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test execute_contract`

### Task 2.4: Implement stable non-execute surfaces and maturity upgrade
- **Location**:
  - `crates/agent-provider-claude/src/adapter.rs`
  - `crates/agent-provider-claude/tests/adapter_contract.rs`
- **Description**: Implement non-stub behavior for `capabilities`, `healthcheck`, `limits`, and
  `auth-state`, then promote `metadata().maturity` from `stub` to `stable` with environment-aware
  readiness semantics.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Capabilities correctly reflect runtime readiness (auth configured, network/client availability).
  - Healthcheck reports deterministic `healthy/degraded/unhealthy` status with actionable summary.
  - Adapter metadata reports `stable` only after required surfaces are implemented.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test adapter_contract`

### Task 2.5: Add fixture-backed contract regression tests
- **Location**:
  - `crates/agent-provider-claude/tests/adapter_contract.rs`
  - `crates/agent-provider-claude/tests/execute_contract.rs`
  - `crates/agent-provider-claude/tests/fixtures/README.md`
- **Description**: Add regression tests that replay fixture payloads and assert exact provider
  envelopes, error codes, and retryability flags across representative scenarios.
- **Dependencies**:
  - Task 2.3
  - Task 2.4
  - Task 1.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover success + at least four failure families (auth, validation, rate-limit, timeout).
  - Regression tests pin stable error codes and categories.
  - Fixture update procedure is documented and followed.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test adapter_contract`
  - `cargo test -p nils-agent-provider-claude --test execute_contract`

## Sprint 3: agentctl integration and codex workflow migration path
**Goal**: Make Claude execution usable through `agentctl` and publish clear codex-to-claude migration guidance.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-agentctl --test provider_registry`
  - `cargo test -p nils-agentctl --test provider_commands`
  - `cargo test -p nils-agentctl --test workflow_run`
  - `cargo test -p nils-agentctl --test diag_capabilities`
  - `cargo test -p nils-agentctl --test diag_doctor`
- Verify: provider commands, diagnostics, and workflow execution recognize Claude as stable and executable.

### Task 3.1: Update provider registry/commands for Claude stable readiness
- **Location**:
  - `crates/agentctl/src/provider/registry.rs`
  - `crates/agentctl/tests/provider_registry.rs`
  - `crates/agentctl/tests/provider_commands.rs`
- **Description**: Update registry expectations and command tests so `claude` is treated as a
  stable built-in provider when adapter prerequisites are satisfied.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Provider list JSON/text reflects `claude` maturity and availability accurately.
  - Unknown-provider and override behavior remains unchanged.
  - Default-provider behavior stays deterministic (`codex` unless explicitly changed).
- **Validation**:
  - `cargo test -p nils-agentctl --test provider_registry`
  - `cargo test -p nils-agentctl --test provider_commands`

### Task 3.2: Add Claude workflow-run execution and failure-path coverage
- **Location**:
  - `crates/agentctl/tests/workflow_run.rs`
  - `crates/agentctl/tests/fixtures/workflow/claude-minimal.json`
- **Description**: Add workflow fixtures and assertions for `provider=claude` success/failure
  paths (auth missing, timeout, provider error mapping), ensuring ledger semantics remain stable.
- **Dependencies**:
  - Task 2.5
- **Complexity**: 7
- **Acceptance criteria**:
  - Workflow provider-step tests pass for claude success and deterministic failure envelopes.
  - Exit code behavior for workflow run remains consistent with existing policy.
  - Artifact and retry semantics remain unchanged.
- **Validation**:
  - `cargo test -p nils-agentctl --test workflow_run`

### Task 3.3: Add codex-cli -> Claude migration mapping runbook
- **Location**:
  - `crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`
  - `crates/agentctl/README.md`
  - `crates/agent-provider-claude/README.md`
- **Description**: Document how codex-cli user intents map to Claude-enabled provider workflows,
  including equivalent commands, semantic differences, unsupported capabilities, and fallback paths.
- **Dependencies**:
  - Task 1.1
  - Task 2.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Runbook includes a complete mapping table covering codex-cli command families.
  - Each unsupported item has a documented alternative or explicit limitation note.
  - Both READMEs link to the runbook.
- **Validation**:
  - `rg -n "codex-cli|claude|exact|semantic|unsupported|fallback" crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`
  - `rg -n "codex-to-claude-mapping" crates/agentctl/README.md crates/agent-provider-claude/README.md`

### Task 3.4: Update diagnostics surfaces for Claude readiness transparency
- **Location**:
  - `crates/agentctl/src/diag/capabilities.rs`
  - `crates/agentctl/src/diag/doctor.rs`
  - `crates/agentctl/tests/diag_capabilities.rs`
  - `crates/agentctl/tests/diag_doctor.rs`
- **Description**: Ensure `diag capabilities` and `diag doctor` surface Claude readiness reasons
  (auth missing, client unavailable, rate-limit degraded) in machine-readable and text modes.
- **Dependencies**:
  - Task 2.4
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - JSON outputs include deterministic checks for Claude provider readiness.
  - Text outputs remain concise and actionable.
  - Existing Codex and automation checks do not regress.
- **Validation**:
  - `cargo test -p nils-agentctl --test diag_capabilities`
  - `cargo test -p nils-agentctl --test diag_doctor`

## Sprint 4: Closed-source verification gate and rollout hardening
**Goal**: Add release gating that proves correctness without requiring Claude source code.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify: required checks pass; coverage gate passes for non-doc changes.

### Task 4.1: Add mock-profile and live-profile verification entrypoints
- **Location**:
  - `crates/agent-provider-claude/tests/live_smoke.rs`
  - `crates/agent-provider-claude/tests/mock_contract.rs`
  - `crates/agent-provider-claude/docs/runbooks/verification-oracles.md`
- **Description**: Split verification into deterministic mock profile (always in CI) and optional
  live profile (`CLAUDE_LIVE_TEST=1`) for periodic contract drift detection.
- **Dependencies**:
  - Task 2.5
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - CI profile does not require network credentials and is fully deterministic.
  - Live profile is opt-in, bounded, and explicitly non-blocking for default CI.
  - Runbook documents when and how to run each profile.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test mock_contract`
  - `CLAUDE_LIVE_TEST=1 cargo test -p nils-agent-provider-claude --test live_smoke -- --ignored`

### Task 4.2: Add black-box characterization runner for Claude CLI parity checks
- **Location**:
  - `scripts/ci/claude-characterization.sh`
  - `crates/agent-provider-claude/tests/fixtures/characterization/claude-cli-smoke.json`
  - `crates/agent-provider-claude/docs/runbooks/verification-oracles.md`
- **Description**: Add a characterization runner that captures `stdout/stderr/exit` from local
  Claude CLI for overlap scenarios and reports diffs against expected adapter envelopes.
- **Dependencies**:
  - Task 1.4
  - Task 4.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Runner gracefully skips when Claude CLI is unavailable.
  - Diff output is machine-readable and references fixture IDs.
  - Characterization results never block deterministic CI by default.
- **Validation**:
  - `bash scripts/ci/claude-characterization.sh --mode mock`
  - `bash scripts/ci/claude-characterization.sh --mode local-cli --allow-skip`
  - `test -f target/claude-characterization/local-cli-report.json`

### Task 4.3: Update dependency/runtime docs and operator guidance
- **Location**:
  - `BINARY_DEPENDENCIES.md`
  - `crates/agentctl/README.md`
  - `crates/agent-provider-claude/README.md`
- **Description**: Update maturity/runtime requirements from stub assumptions to stable-implementation
  requirements, including required credentials, optional local Claude CLI, and degradation behavior.
- **Dependencies**:
  - Task 3.1
  - Task 4.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `BINARY_DEPENDENCIES.md` reflects Claude stable runtime expectations.
  - Operator docs clearly separate required vs optional dependencies.
  - No stale references remain to “compile-only stub” for Claude.
- **Validation**:
  - `rg -n "agent-provider-claude|claude|stub|stable|runtime requirement" BINARY_DEPENDENCIES.md crates/agentctl/README.md crates/agent-provider-claude/README.md`

### Task 4.4: Run required checks and coverage gate before delivery
- **Location**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `target/coverage/lcov.info`
- **Description**: Run repository-required checks and coverage gate; if failures occur, capture root
  causes and address before rollout.
- **Dependencies**:
  - Task 3.4
  - Task 4.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Required checks entrypoint succeeds.
  - Coverage remains >= 85.00% for non-doc changes.
  - Any skipped checks are explicitly justified and tracked.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Testing Strategy
- Unit:
  - Adapter config parsing, error mapping, retry policy, and prompt templating.
- Integration:
  - Provider contract tests + agentctl provider/diag/workflow integration tests.
- E2E/manual:
  - Optional live Claude API smoke tests and local Claude CLI characterization runner.
- Regression:
  - Fixture-backed snapshots for provider envelopes and stable error codes.

## Risks & gotchas
- Claude API contract drift could silently change behavior; mitigate with live drift checks and
  fixture versioning.
- Overfitting to local Claude CLI behavior could conflict with official API; mitigate with oracle
  priority rules.
- Cross-provider parity expectations can be unrealistic for Codex-only features; mitigate with
  explicit `unsupported` mappings and deterministic fallback behavior.
- Secret leakage risk in logs/fixtures; mitigate with strict redaction and fixture audits.

## Rollback plan
- Keep rollout behind an adapter maturity gate:
  - If critical regressions are found, set Claude metadata back to `stub` and disable execute
    capability in `capabilities`.
- Revert provider integration expectations in `agentctl` tests/docs to stub-safe defaults.
- Preserve released docs/runbooks; add an incident note describing why rollout was reverted and what
  validation gap was discovered.
- Continue shipping deterministic mock tests while live/profile checks are repaired before next
  promotion attempt.
