# Plan: Extract codex-core and decouple codex runtime from codex-cli

## Overview
This plan introduces a new `codex-core` crate as the reusable runtime layer for Codex-specific
non-UI logic, then rewires both `codex-cli` and `agent-provider-codex` to depend on that shared
core instead of depending on each other. The target architecture is: `codex-core` owns runtime
primitives (`auth/path/config`, execute policy wrapper, typed errors), `codex-cli` owns user-facing
CLI UX, and `agent-provider-codex` only performs provider contract mapping.

## Scope
- In scope:
  - Add a new `crates/codex-core` crate and register it in workspace metadata.
  - Move reusable runtime logic from `codex-cli` into `codex-core` without changing behavior.
  - Replace `agent-provider-codex -> codex-cli` dependency with `agent-provider-codex -> codex-core`.
  - Keep `codex-cli` focused on command parsing, help text, output rendering, and compatibility hints.
  - Preserve provider contract behavior and existing exit semantics.
- Out of scope:
  - New user-facing command families in `codex-cli`.
  - Any behavior change to `codex-cli` JSON schemas (`codex-cli.auth.v1`, `codex-cli.diag.rate-limits.v1`).
  - Refactoring `rate_limits` and `starship` into core during this iteration.
  - Introducing `claude-cli` in the same change set.

## Assumptions (if any)
1. `provider-adapter.v1` behavior in `agent-provider-codex` must remain functionally stable.
2. `codex-core` can depend on `nils-common` and `serde_json` but must not own CLI parsing.
3. Existing `codex-cli` and `agent-provider-codex` tests are strong enough to catch regressions if
   extraction is done incrementally.
4. Runtime text that is user-facing remains in `codex-cli`; core error types are stable and mappable.

## Success Criteria
1. `agent-provider-codex/Cargo.toml` no longer depends on `nils-codex-cli`; it depends on
   `nils-codex-core`.
2. `codex-cli` continues passing existing command/contract tests with unchanged user-visible behavior.
3. `agent-provider-codex` continues passing adapter contract tests with unchanged categories/codes.
4. Full required checks and coverage gate pass after refactor.

## Parallelization notes
- After Task 1.2 lands, Task 1.3 and Task 1.4 can run in parallel.
- After Task 2.1 and Task 2.2 land, Task 2.3 can start; Task 3.1 starts after Task 2.3.
- In Sprint 4, docs tasks can run in parallel with rollout checklist drafting, but before final
  required checks.

## Sprint 1: Architecture baseline and crate foundation
**Goal**: Freeze boundaries and create a minimal compilable `codex-core` skeleton before moving logic.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/codex-core-runtime-decoupling-plan.md`
  - `cargo test -p nils-codex-cli --test paths`
- Verify: plan is executable and baseline behavior is captured before extraction.

### Task 1.1: Define codex-core ownership and API boundary
- **Location**:
  - `crates/codex-core/docs/specs/codex-core-boundary-v1.md`
  - `crates/codex-cli/README.md`
  - `crates/agent-provider-codex/README.md`
- **Description**: Document exact ownership split: what moves to `codex-core`, what remains in
  `codex-cli`, and what `agent-provider-codex` is allowed to call. Include anti-goals to prevent
  core from growing UI concerns.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Boundary doc lists module-level ownership (`auth/path/config/exec/error` in core, CLI UX in
    `codex-cli`).
  - Docs explicitly forbid Clap/UI output handling in `codex-core`.
  - `agent-provider-codex` role is stated as provider contract mapping only.
- **Validation**:
  - `rg -n "auth|path|config|exec|typed error|UI|Clap|provider contract mapping" crates/codex-core/docs/specs/codex-core-boundary-v1.md`

### Task 1.2: Scaffold codex-core crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/codex-core/Cargo.toml`
  - `crates/codex-core/src/lib.rs`
  - `crates/codex-core/README.md`
  - `crates/codex-core/docs/README.md`
- **Description**: Add the new crate to workspace members with baseline module exports and docs
  index. Ensure crate compiles with no moved logic yet.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace includes `crates/codex-core`.
  - `cargo check -p nils-codex-core` succeeds.
  - README describes runtime-only ownership and non-goals.
- **Validation**:
  - `cargo check -p nils-codex-core`
  - `rg -n "codex-core|runtime|non-UI" crates/codex-core/README.md`

### Task 1.3: Add typed error model and mapping helpers in codex-core
- **Location**:
  - `crates/codex-core/src/error.rs`
  - `crates/codex-core/src/lib.rs`
  - `crates/codex-core/tests/error_contract.rs`
- **Description**: Create stable typed errors for runtime operations (config/auth/exec/dependency/
  validation) and helper conversions for both CLI and provider layers.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Error enum covers runtime categories needed by both consumers.
  - Conversion helpers preserve actionable context while avoiding secret leakage.
  - Error contract tests lock codes/categories and retryability metadata where relevant.
- **Validation**:
  - `cargo test -p nils-codex-core --test error_contract`

### Task 1.4: Add characterization tests for current codex runtime behavior
- **Location**:
  - `crates/codex-cli/tests/paths.rs`
  - `crates/codex-cli/tests/jwt.rs`
  - `crates/agent-provider-codex/tests/adapter_contract.rs`
- **Description**: Expand or pin baseline tests around currently shared runtime behavior so
  extraction can prove no semantic changes.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Baseline tests clearly assert current auth/path/execute-policy behavior.
  - Tests fail on unintended behavior drift during extraction.
  - No new network dependencies are introduced.
- **Validation**:
  - `cargo test -p nils-codex-cli --test paths --test jwt`
  - `cargo test -p nils-agent-provider-codex --test adapter_contract`

## Sprint 2: Move reusable runtime logic into codex-core
**Goal**: Relocate reusable non-UI code from `codex-cli` into `codex-core` while keeping outputs stable.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-codex-core`
  - `cargo test -p nils-codex-cli`
- Verify: codex core tests pass and codex CLI behavior remains unchanged.

### Task 2.1: Extract path and config primitives
- **Location**:
  - `crates/codex-cli/src/paths.rs`
  - `crates/codex-cli/src/config.rs`
  - `crates/codex-core/src/paths.rs`
  - `crates/codex-core/src/config.rs`
  - `crates/codex-core/tests/paths_config_contract.rs`
- **Description**: Move path and config resolution logic into core modules and expose stable APIs.
  Keep codex-cli wrappers thin to avoid command behavior changes.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `codex-cli` uses `codex-core` for path/config lookups.
  - Existing env-variable precedence and defaults remain unchanged.
  - Core-level tests cover default + override + invalid input cases.
- **Validation**:
  - `cargo test -p nils-codex-core --test paths_config_contract`
  - `cargo test -p nils-codex-cli --test paths --test config`

### Task 2.2: Extract auth parsing and identity helpers
- **Location**:
  - `crates/codex-cli/src/auth/mod.rs`
  - `crates/codex-cli/src/jwt.rs`
  - `crates/codex-core/src/auth.rs`
  - `crates/codex-core/src/jwt.rs`
  - `crates/codex-core/tests/auth_contract.rs`
- **Description**: Move auth-file decoding and identity extraction helpers into core while
  preserving current secret-file parsing semantics and error behavior.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
- **Complexity**: 8
- **Acceptance criteria**:
  - Auth decoding functions used by both CLI and provider are sourced from core.
  - Existing identity precedence (`email -> identity -> account_id`) remains intact.
  - Invalid auth file errors remain deterministic and non-secret-leaking.
- **Validation**:
  - `cargo test -p nils-codex-core --test auth_contract`
  - `cargo test -p nils-codex-cli --test jwt --test auth_current_sync`

### Task 2.3: Extract execute policy wrapper and dangerous-mode checks
- **Location**:
  - `crates/codex-cli/src/agent/exec.rs`
  - `crates/codex-core/src/exec.rs`
  - `crates/codex-core/tests/exec_contract.rs`
- **Description**: Move runtime execute wrapper logic (dangerous-mode gate, codex binary presence,
  stderr capture helpers) into core with typed error surfaces.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Core execute wrapper exposes APIs needed by CLI and provider callers.
  - Policy gate behavior (`CODEX_ALLOW_DANGEROUS_ENABLED`) remains unchanged.
  - Contract tests cover missing binary, disabled policy, and success/failure execution paths.
- **Validation**:
  - `cargo test -p nils-codex-core --test exec_contract`
  - `cargo test -p nils-codex-cli --test agent_exec --test agent_prompt`

### Task 2.4: Slim codex-cli to UX orchestration only
- **Location**:
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/src/lib.rs`
  - `crates/codex-cli/src/agent/mod.rs`
  - `crates/codex-cli/src/agent/exec.rs`
  - `crates/codex-cli/src/auth/mod.rs`
  - `crates/codex-cli/src/auth/current.rs`
- **Description**: Update codex-cli modules to call core APIs and keep only UX concerns: command
  routing, help/usage, compatibility redirects, and output rendering.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - `codex-cli` no longer owns duplicated runtime primitives moved to core.
  - User-facing text, exit codes, and command surface stay stable.
  - CLI still exports completion and legacy redirect hints unchanged.
- **Validation**:
  - `cargo test -p nils-codex-cli --test main_entrypoint --test dispatch --test completion_contract`

## Sprint 3: Decouple agent-provider-codex from codex-cli
**Goal**: Make provider adapter depend only on runtime core and provider contract crates.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-agent-provider-codex`
  - `cargo test -p nils-agentctl --test provider_registry --test provider_commands`
- Verify: provider behavior is stable and registry integration is unaffected.

### Task 3.1: Replace codex-cli dependency with codex-core in provider crate
- **Location**:
  - `crates/agent-provider-codex/Cargo.toml`
  - `crates/agent-provider-codex/src/adapter.rs`
  - `crates/agent-provider-codex/src/lib.rs`
- **Description**: Repoint imports from `codex_cli::{agent, auth, paths}` to corresponding
  `codex_core` modules and remove crate-level coupling to CLI implementation.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - `agent-provider-codex` compiles without `nils-codex-cli` dependency.
  - Provider adapter behavior remains equivalent for execute/auth/healthcheck.
  - No CLI parsing or UX helpers are imported into provider crate.
- **Validation**:
  - `cargo check -p nils-agent-provider-codex`
  - `if rg -n "codex_cli::" crates/agent-provider-codex/src; then echo "unexpected codex_cli import in provider" && exit 1; fi`

### Task 3.2: Rebaseline provider contract and error mapping tests
- **Location**:
  - `crates/agent-provider-codex/tests/adapter_contract.rs`
  - `crates/agent-provider-codex/tests/dependency_boundary.rs`
- **Description**: Revalidate adapter outputs after dependency inversion, ensuring category/code,
  health summary, and capability flags remain consistent.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Existing provider contract tests pass without behavioral diffs.
  - Any intentional message-level changes are documented and minimal.
  - Auth-file parse failures remain mapped to auth category with stable code.
- **Validation**:
  - `cargo test -p nils-agent-provider-codex --test adapter_contract`

### Task 3.3: Validate agentctl compatibility against refactored codex provider
- **Location**:
  - `crates/agentctl/tests/provider_registry.rs`
  - `crates/agentctl/tests/provider_commands.rs`
  - `crates/agentctl/tests/workflow_run.rs`
- **Description**: Ensure provider-neutral orchestration continues to resolve and execute codex
  provider normally after dependency graph changes.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Provider list and healthcheck tests still pass with codex provider default semantics.
  - Workflow provider-step execution for codex remains unchanged.
  - No regressions in provider selection precedence.
- **Validation**:
  - `cargo test -p nils-agentctl --test provider_registry --test provider_commands --test workflow_run`

### Task 3.4: Add architecture guardrails against re-coupling
- **Location**:
  - `crates/agent-provider-codex/tests/dependency_boundary.rs`
  - `scripts/ci/codex-core-boundary-check.sh`
- **Description**: Add a lightweight boundary test or check to prevent future reintroduction of
  `codex-cli` imports into provider runtime crates.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 4
- **Acceptance criteria**:
  - CI-visible check fails if provider imports `codex_cli`.
  - Rule is documented so contributors understand the boundary.
  - Check is deterministic and fast.
- **Validation**:
  - `cargo test -p nils-agent-provider-codex --test dependency_boundary`

## Sprint 4: Docs, rollout hardening, and release gate
**Goal**: Publish the new ownership model and complete full delivery validation.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify: full repository checks are green and coverage remains above policy threshold.

### Task 4.1: Update ownership and dependency documentation
- **Location**:
  - `crates/codex-cli/README.md`
  - `crates/agent-provider-codex/README.md`
  - `crates/codex-core/README.md`
  - `BINARY_DEPENDENCIES.md`
- **Description**: Update docs to reflect new layered architecture and runtime dependency paths.
  Clarify that codex provider uses core runtime primitives rather than CLI internals.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - All relevant READMEs describe `codex-core` as reusable runtime layer.
  - No stale text claims provider depends on CLI crate internals.
  - Binary/runtime requirement docs remain accurate and actionable.
- **Validation**:
  - `rg -n "codex-core|runtime layer|provider contract mapping|codex-cli" crates/codex-cli/README.md crates/agent-provider-codex/README.md crates/codex-core/README.md BINARY_DEPENDENCIES.md`

### Task 4.2: Add migration runbook for crate consumers
- **Location**:
  - `docs/runbooks/codex-core-migration.md`
  - `crates/codex-core/README.md`
- **Description**: Provide a migration guide for internal contributors and downstream crate users:
  old import paths, new import paths, and compatibility notes for incremental migration.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Runbook includes before/after import examples for provider and CLI call sites.
  - Guide documents compatibility expectations and phased adoption order.
  - Core README links to runbook.
- **Validation**:
  - `rg -n "before|after|codex_cli|codex_core|migration" docs/runbooks/codex-core-migration.md`
  - `rg -n "codex-core-migration" crates/codex-core/README.md`

### Task 4.3: Execute full required checks and coverage gate
- **Location**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `target/coverage/lcov.info`
- **Description**: Run required repository checks and coverage gate after refactor completion; fix
  any regressions before delivery.
- **Dependencies**:
  - Task 2.4
  - Task 3.4
  - Task 4.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Required checks pass end-to-end.
  - Coverage remains `>= 85.00%`.
  - Any temporary skips are documented with concrete remediation owner/date.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

### Task 4.4: Delivery checklist for feature PR flow
- **Location**:
  - `docs/plans/codex-core-runtime-decoupling-plan.md`
  - `.agents/skills/workflows/pr/feature/deliver-feature-pr/SKILL.md`
- **Description**: Attach explicit delivery checklist for branch/PR/CI/merge sequence so execution
  phase can follow deterministic `deliver-feature-pr` flow with no ambiguous scope decisions.
- **Dependencies**:
  - Task 4.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Checklist includes preflight, create PR, wait-ci, close, and post-merge verification.
  - Checklist references required checks and coverage evidence paths.
  - Execution phase can be delegated without missing release gates.
- **Validation**:
  - `plan-tooling to-json --file docs/plans/codex-core-runtime-decoupling-plan.md --pretty >/dev/null`

#### Delivery checklist (`deliver-feature-pr`)
- [ ] `deliver-feature-pr.sh preflight --base main`
- [ ] Create branch + PR via `create-feature-pr` flow (`feat/*` -> `main`) with this plan linked.
- [ ] Run required checks and coverage evidence before CI wait:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- [ ] `deliver-feature-pr.sh wait-ci --pr <number>` until all checks are green (fix + push on failure).
- [ ] `deliver-feature-pr.sh close --pr <number>` to mark ready (if draft), merge, and cleanup.
- [ ] Post-merge verification artifacts captured in PR notes:
  - PR URL
  - CI green summary
  - merge commit SHA
  - coverage summary path (`target/coverage/lcov.info`)

## Testing Strategy
- Unit:
  - `codex-core` tests for config/path/auth/exec/error contracts.
- Integration:
  - `agent-provider-codex` adapter contract tests and `agentctl` provider/workflow tests.
- E2E/manual:
  - Targeted `codex-cli` command smoke tests for unchanged UX behavior.
- Regression:
  - Existing codex-cli auth/diag JSON contract tests and provider error mapping tests.

## Risks & gotchas
- Runtime and UX logic can accidentally be split at the wrong layer, causing message drift.
- Shared-core extraction can introduce circular dependency pressure if module boundaries are not strict.
- Subtle auth-path precedence differences may pass compile checks but break user workflows.
- Large refactor can hide behavior changes unless characterization tests are pinned first.

## Rollback plan
- Keep extraction incremental by commits per sprint; if regressions appear, revert only the latest
  migration commit while preserving prior boundary docs/tests.
- If provider behavior regresses, temporarily restore `agent-provider-codex -> codex-cli`
  dependency and keep `codex-core` experimental behind docs-only status until gaps are fixed.
- If CLI UX regresses, route affected call sites back to existing codex-cli local implementations
  while retaining core APIs for non-user-facing paths.
- Record rollback trigger and restoration commands in PR notes before reattempting migration.
