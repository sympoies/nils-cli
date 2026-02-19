# Plan: Claude core and CLI unified implementation

## Overview
This plan defines a single execution track for Claude support across both core runtime and CLI
surfaces. The core implementation lives in `agent-provider-claude` (contract behavior, execution,
limits, readiness), and CLI implementation lives in `agentctl` (provider commands, diagnostics,
workflow behavior, operator-facing docs). The structure mirrors the decoupling-plan rigor: clear
boundaries first, then core implementation, then CLI integration, then release gating.

## Scope
- In scope:
  - Finalize Claude core behavior under `provider-adapter.v1` in `agent-provider-claude`.
  - Finalize Claude CLI behavior in `agentctl` for provider listing, diagnostics, and workflows.
  - Align docs, runbooks, fixtures, and CI verification paths for deterministic release confidence.
  - Keep core and CLI implementation tasks in one plan with explicit dependencies.
- Out of scope:
  - Introduce a new standalone `claude-cli` binary in this iteration.
  - Change provider-neutral schema ownership in `agent-runtime-core` beyond compatibility updates.
  - Rework Codex/Gemini feature semantics outside Claude-related integration points.

## Assumptions (if any)
1. `provider-adapter.v1` in `crates/agent-runtime-core` remains the authoritative integration
   contract for provider adapters and `agentctl`.
2. `agent-provider-claude` owns Claude runtime behavior; `agentctl` owns user-facing CLI output and
   command routing.
3. Live network verification stays opt-in; deterministic fixture-based checks are mandatory for CI.
4. Existing Claude fixture assets are sufficient to expand contract coverage without introducing
   secrets.

## Success Criteria
1. Claude core surfaces (`execute`, `auth-state`, `capabilities`, `healthcheck`, `limits`) are
   fully specified and covered by deterministic contract tests.
2. `agentctl` supports Claude as a stable provider in provider listing, diagnostics, and workflow
   execution paths with deterministic success/failure behavior.
3. Core/CLI migration and operator docs clearly describe required vs optional runtime dependencies.
4. Required checks and coverage gate pass after implementation.

## Parallelization notes
- After Task 1.2 lands, Task 1.3 and Task 1.4 can run in parallel.
- After Task 2.2 lands, Task 2.3 and Task 2.4 can run in parallel.
- In Sprint 3, Task 3.2 and Task 3.3 can run in parallel after Task 3.1.
- In Sprint 4, docs updates and rollout checklist preparation can run in parallel before final
  required checks.

## Sprint 1: Contract and boundary baseline
**Goal**: Freeze ownership boundaries and validation rules before changing behavior.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/claude-core-cli-unified-implementation-plan.md`
  - `plan-tooling to-json --file docs/plans/claude-core-cli-unified-implementation-plan.md --pretty >/dev/null`
- Verify: plan structure is executable and dependency graph resolves.

### Task 1.1: Define Claude core vs CLI ownership boundary
- **Location**:
  - `crates/agent-provider-claude/docs/specs/claude-provider-contract-v1.md`
  - `crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`
  - `crates/agent-provider-claude/README.md`
  - `crates/agentctl/README.md`
- **Description**: Document and align module ownership so contributors can clearly distinguish core
  runtime responsibilities from CLI UX responsibilities. Include anti-goals preventing core from
  absorbing CLI-specific formatting/routing concerns.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Core ownership (`provider contract`, `error mapping`, `execution policy`) is explicit.
  - CLI ownership (`provider command UX`, `diag output`, `workflow reporting`) is explicit.
  - Anti-goals explicitly disallow moving CLI rendering concerns into provider core modules.
- **Validation**:
  - `rg -n "ownership|core|CLI|provider contract|diag|workflow|anti-goal" crates/agent-provider-claude/docs/specs/claude-provider-contract-v1.md crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`

### Task 1.2: Rebaseline Claude contract and parity matrix for implementation work
- **Location**:
  - `crates/agent-provider-claude/docs/specs/claude-provider-contract-v1.md`
  - `crates/agent-provider-claude/docs/specs/codex-cli-claude-parity-matrix-v1.md`
  - `crates/agent-runtime-core/src/schema.rs`
- **Description**: Ensure contract and parity docs fully capture Claude behavior needed by both core
  and CLI implementation, including stable error categories/codes, retryability, and unsupported
  behavior handling.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Contract covers all provider operations with explicit behavior and error taxonomy.
  - Parity matrix classifies each codex-oriented surface as exact/semantic/unsupported.
  - Contract language is compatible with current `provider-adapter.v1` schema expectations.
- **Validation**:
  - `rg -n "capabilities|healthcheck|execute|limits|auth-state|retryable|compatibility" crates/agent-provider-claude/docs/specs/claude-provider-contract-v1.md`
  - `rg -n "exact|semantic|unsupported|agent|auth|diag|config|starship" crates/agent-provider-claude/docs/specs/codex-cli-claude-parity-matrix-v1.md`

### Task 1.3: Strengthen deterministic fixture and oracle policy
- **Location**:
  - `crates/agent-provider-claude/tests/fixtures/README.md`
  - `crates/agent-provider-claude/tests/fixtures/characterization/manifest.json`
  - `crates/agent-provider-claude/docs/runbooks/verification-oracles.md`
- **Description**: Lock fixture IDs, update policy for mock/live characterization, and define
  redaction and update rules so core and CLI validation can rely on shared deterministic artifacts.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Fixture manifest includes success/auth/rate-limit/timeout/malformed-response families.
  - Runbook defines priority between API contract, characterization, and fixture tests.
  - Secret scanning guidance is explicit for fixture updates.
- **Validation**:
  - `rg -n "\"id\"\\s*:\\s*\"(success|auth_failure|rate_limit|timeout|malformed_response)\"" crates/agent-provider-claude/tests/fixtures/characterization/manifest.json`
  - `rg -n "Primary oracle|Secondary oracle|Tertiary oracle|release gate|redaction" crates/agent-provider-claude/docs/runbooks/verification-oracles.md`
  - `rg -n "secret scan|redaction guidance|fixture update|secret leakage" crates/agent-provider-claude/docs/runbooks/verification-oracles.md crates/agent-provider-claude/tests/fixtures/README.md`

### Task 1.4: Prepare CLI-facing mapping baseline for migration safety
- **Location**:
  - `crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`
  - `crates/agentctl/README.md`
  - `crates/agent-provider-claude/README.md`
- **Description**: Reconcile CLI migration guidance with current contract/parity definitions so
  implementation tasks have one authoritative mapping for expected command behavior.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Mapping runbook lists command family mapping and unsupported behavior alternatives.
  - Both READMEs reference the mapping runbook.
  - Migration notes remain consistent with parity matrix classifications.
- **Validation**:
  - `rg -n "codex-cli|claude|unsupported|fallback|semantic" crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`
  - `rg -n "codex-to-claude-mapping" crates/agentctl/README.md crates/agent-provider-claude/README.md`

## Sprint 2: Claude core implementation in agent-provider-claude
**Goal**: Deliver stable Claude provider runtime behavior with deterministic test coverage.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-agent-provider-claude --test config_contract`
  - `cargo test -p nils-agent-provider-claude --test client_contract`
  - `cargo test -p nils-agent-provider-claude --test execute_contract`
  - `cargo test -p nils-agent-provider-claude --test adapter_contract`
- Verify: core runtime behavior is implemented and validated without CLI coupling.

### Task 2.1: Finalize Claude config and auth-state primitives
- **Location**:
  - `crates/agent-provider-claude/src/config.rs`
  - `crates/agent-provider-claude/src/lib.rs`
  - `crates/agent-provider-claude/tests/config_contract.rs`
- **Description**: Harden configuration parsing, auth resolution, and redaction-safe diagnostics for
  deterministic readiness evaluation across local and CI environments.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Config resolution is deterministic for required and optional settings.
  - Auth-state exposes actionable failures without leaking credentials.
  - Contract tests cover missing/invalid values and override precedence.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test config_contract`

### Task 2.2: Finalize Claude API client behavior and error mapping
- **Location**:
  - `crates/agent-provider-claude/src/client.rs`
  - `crates/agent-provider-claude/src/adapter.rs`
  - `crates/agent-provider-claude/tests/client_contract.rs`
- **Description**: Ensure deterministic timeout/retry policy and stable translation of API/network
  failures into provider contract categories and codes.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Timeout/network/4xx/5xx flows map to stable categories and codes.
  - Retry applies only to documented retryable conditions.
  - Client tests cover both retryable and non-retryable branches.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test client_contract`

### Task 2.3: Finalize execute flow and prompt intent mapping
- **Location**:
  - `crates/agent-provider-claude/src/adapter.rs`
  - `crates/agent-provider-claude/src/prompts.rs`
  - `crates/agent-provider-claude/tests/execute_contract.rs`
- **Description**: Complete `execute` behavior for intent families (`prompt`, `advice`,
  `knowledge`) with stable envelope formatting and deterministic validation of invalid inputs.
- **Dependencies**:
  - Task 2.2
  - Task 1.4
- **Complexity**: 8
- **Acceptance criteria**:
  - Execute path returns stable success/failure envelopes for covered intents.
  - Input validation behavior is deterministic and contract-compliant.
  - Prompt mapping remains traceable to parity/migration guidance.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test execute_contract`

### Task 2.4: Finalize non-execute surfaces and readiness semantics
- **Location**:
  - `crates/agent-provider-claude/src/adapter.rs`
  - `crates/agent-provider-claude/tests/adapter_contract.rs`
  - `crates/agent-provider-claude/tests/mock_contract.rs`
- **Description**: Ensure `capabilities`, `healthcheck`, `limits`, and `auth-state` expose stable,
  environment-aware readiness semantics and deterministic degraded/error reporting.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Readiness and degraded reasons are stable across deterministic test scenarios.
  - Limits/capabilities payloads are contract-compliant and actionable.
  - Mock contract tests cover stable behavior under missing auth and failure injection.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test adapter_contract`
  - `cargo test -p nils-agent-provider-claude --test mock_contract`

### Task 2.5: Add boundary regression tests to prevent CLI coupling
- **Location**:
  - `crates/agent-provider-claude/tests/adapter_contract.rs`
  - `crates/agent-provider-claude/src/lib.rs`
  - `crates/agent-provider-claude/Cargo.toml`
- **Description**: Add checks that keep Claude core implementation independent from CLI parsing and
  rendering concerns, ensuring provider code remains reusable as runtime core.
- **Dependencies**:
  - Task 2.3
  - Task 2.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Provider crate has no dependency on CLI crates for behavior logic.
  - Contract tests continue passing after boundary guard updates.
  - Boundary constraints are documented in crate docs.
- **Validation**:
  - `cargo check -p nils-agent-provider-claude`
  - `if rg -n "nils-agentctl|clap::" crates/agent-provider-claude/src; then echo "unexpected CLI coupling in provider core" && exit 1; fi`
  - `rg -n "boundary|CLI coupling|provider core|runtime behavior" crates/agent-provider-claude/README.md crates/agent-provider-claude/docs/specs/claude-provider-contract-v1.md`

## Sprint 3: Claude CLI implementation in agentctl
**Goal**: Deliver stable Claude experience in provider commands, diagnostics, and workflows.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-agentctl --test provider_registry`
  - `cargo test -p nils-agentctl --test provider_commands`
  - `cargo test -p nils-agentctl --test workflow_run`
  - `cargo test -p nils-agentctl --test diag_capabilities`
  - `cargo test -p nils-agentctl --test diag_doctor`
- Verify: CLI behavior is deterministic for Claude across provider, workflow, and diag surfaces.

### Task 3.1: Finalize provider registry and command surfaces for Claude
- **Location**:
  - `crates/agentctl/src/provider/registry.rs`
  - `crates/agentctl/src/provider/commands.rs`
  - `crates/agentctl/tests/provider_registry.rs`
  - `crates/agentctl/tests/provider_commands.rs`
- **Description**: Ensure provider commands and listing surfaces treat Claude as a stable built-in
  provider while preserving default-provider behavior and override semantics.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Provider list output reflects Claude availability and readiness reasons.
  - Unknown-provider and override behavior remain deterministic.
  - Existing codex default semantics remain unchanged unless explicitly configured.
- **Validation**:
  - `cargo test -p nils-agentctl --test provider_registry`
  - `cargo test -p nils-agentctl --test provider_commands`

### Task 3.2: Finalize workflow execution paths for Claude
- **Location**:
  - `crates/agentctl/src/workflow/run.rs`
  - `crates/agentctl/tests/workflow_run.rs`
  - `crates/agentctl/tests/fixtures/workflow/claude-minimal.json`
- **Description**: Complete workflow-run behavior for Claude provider steps, including deterministic
  success/failure handling and stable artifact/exit semantics.
- **Dependencies**:
  - Task 3.1
  - Task 2.5
- **Complexity**: 7
- **Acceptance criteria**:
  - Workflow tests cover success, missing-auth, and provider-error flows for Claude.
  - Exit-code and ledger behavior remain consistent with existing workflow policy.
  - Fixture-driven tests are deterministic in CI.
- **Validation**:
  - `cargo test -p nils-agentctl --test workflow_run`

### Task 3.3: Finalize diagnostic transparency for Claude readiness
- **Location**:
  - `crates/agentctl/src/diag/capabilities.rs`
  - `crates/agentctl/src/diag/doctor.rs`
  - `crates/agentctl/tests/diag_capabilities.rs`
  - `crates/agentctl/tests/diag_doctor.rs`
- **Description**: Ensure diagnostics report Claude readiness/failure reasons in both human and
  machine-readable modes without regressing other provider diagnostics.
- **Dependencies**:
  - Task 3.1
  - Task 2.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Diagnostic JSON output includes stable readiness reason fields for Claude.
  - Text mode remains concise and actionable.
  - Existing diagnostics for non-Claude providers stay unchanged.
- **Validation**:
  - `cargo test -p nils-agentctl --test diag_capabilities`
  - `cargo test -p nils-agentctl --test diag_doctor`

### Task 3.4: Align CLI docs and completion/help expectations for Claude
- **Location**:
  - `crates/agentctl/src/cli.rs`
  - `crates/agentctl/README.md`
  - `crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`
  - `crates/agentctl/tests/dispatch.rs`
- **Description**: Keep CLI command documentation/help and dispatch behavior aligned with implemented
  Claude support so operator guidance matches runtime behavior.
- **Dependencies**:
  - Task 3.1
  - Task 1.4
- **Complexity**: 5
- **Acceptance criteria**:
  - CLI docs describe Claude usage and limitations consistently.
  - Dispatch test coverage reflects documented command behavior.
  - No stale references to stub-era behavior remain in agentctl docs.
- **Validation**:
  - `cargo test -p nils-agentctl --test dispatch`
  - `rg -n "claude|stable|workflow|provider|mapping" crates/agentctl/README.md crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`

## Sprint 4: Verification gate, rollout hardening, and release readiness
**Goal**: Lock release confidence with deterministic checks and operational documentation.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify: end-to-end checks pass and coverage policy stays compliant.

### Task 4.1: Finalize mock/live Claude verification entrypoints
- **Location**:
  - `crates/agent-provider-claude/tests/mock_contract.rs`
  - `crates/agent-provider-claude/tests/live_smoke.rs`
  - `crates/agent-provider-claude/docs/runbooks/verification-oracles.md`
- **Description**: Keep deterministic mock profile mandatory and live profile opt-in for drift
  detection, with explicit runbook criteria for pass/fail and release blocking rules.
- **Dependencies**:
  - Task 2.5
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Mock profile is CI-stable without network credentials.
  - Live profile remains opt-in and clearly documented.
  - Runbook lists release-blocking mismatch categories.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test mock_contract`
  - `CLAUDE_LIVE_TEST=1 cargo test -p nils-agent-provider-claude --test live_smoke -- --ignored`

### Task 4.2: Finalize characterization runner and reporting outputs
- **Location**:
  - `scripts/ci/claude-characterization.sh`
  - `crates/agent-provider-claude/tests/fixtures/characterization/claude-cli-smoke.json`
  - `crates/agent-provider-claude/docs/runbooks/verification-oracles.md`
- **Description**: Ensure characterization runner behavior is deterministic in mock mode, skip-safe
  when local CLI is unavailable, and produces machine-readable diff/report outputs.
- **Dependencies**:
  - Task 4.1
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Mock mode runs in CI without external CLI dependency.
  - Local CLI mode can skip gracefully while preserving report structure.
  - Runbook documents command examples and output file expectations.
- **Validation**:
  - `bash scripts/ci/claude-characterization.sh --mode mock`
  - `bash scripts/ci/claude-characterization.sh --mode local-cli --allow-skip`
  - `rg -n "claude-characterization.sh|mode mock|mode local-cli|report" crates/agent-provider-claude/docs/runbooks/verification-oracles.md`

### Task 4.3: Finalize runtime dependency and operator documentation
- **Location**:
  - `BINARY_DEPENDENCIES.md`
  - `README.md`
  - `crates/agent-provider-claude/README.md`
  - `crates/agentctl/README.md`
- **Description**: Synchronize runtime dependency and maturity docs so root and crate-level docs
  consistently reflect Claude stable implementation and required credentials.
- **Dependencies**:
  - Task 3.4
  - Task 4.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Root and crate docs agree on Claude maturity and runtime requirements.
  - Required vs optional dependencies are clearly separated.
  - No stale references remain to compile-only Claude behavior.
- **Validation**:
  - `rg -n "agent-provider-claude|claude|stable|runtime requirement|optional" BINARY_DEPENDENCIES.md README.md crates/agent-provider-claude/README.md crates/agentctl/README.md`

### Task 4.4: Execute full required checks and coverage gate before delivery
- **Location**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `target/coverage/lcov.info`
  - `docs/plans/claude-core-cli-unified-implementation-plan.md`
- **Description**: Run final required checks and coverage gate, then lock implementation outcomes
  back into this plan for traceable release readiness.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
  - Task 4.2
  - Task 4.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Required checks pass with no unresolved failures.
  - Coverage remains `>= 85.00%` for non-doc changes.
  - Any temporary skips are documented with owner and follow-up command.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Testing Strategy
- Unit:
  - Claude adapter config/client/execute/non-execute contract tests.
- Integration:
  - `agentctl` provider, diagnostics, and workflow tests for Claude paths.
- E2E/manual:
  - Optional live Claude profile plus local CLI characterization checks.
- Regression:
  - Fixture-backed contract assertions and dependency-boundary checks.

## Risks & gotchas
- API contract drift can break deterministic assumptions if fixture refresh cadence is weak.
- CLI output and docs can drift from core behavior when updates happen in different PRs.
- Retry/readiness semantics can regress subtly if error mapping rules are not pinned by tests.
- Secret leakage risk in fixtures/logs remains high without strict redaction checks.

## Rollback plan
- Keep implementation changes segmented by sprint-scoped commits so regressions can be reverted
  without dropping all Claude progress.
- If core behavior regresses, temporarily downgrade Claude execute availability while preserving
  diagnostics and mapping docs for visibility.
- If CLI integration regresses, keep core adapter stable and gate CLI paths behind provider
  readiness checks until fixes land.
- Record rollback trigger, impacted commands, and restoration validation commands in PR notes before
  reattempting rollout.
