# Plan: Unify codex/claude CLI + core architecture with agentctl compatibility

## Overview
This plan standardizes codex and claude delivery around one architecture: provider runtime in
provider-specific `*-core` crates, user-facing commands in provider-specific `*-cli` crates, and
provider-neutral orchestration in `agentctl` via `agent-provider-*` adapters. The plan keeps existing `codex-cli` product contracts
stable, adds first-class `claude-cli` for users, extracts reusable Claude runtime logic into
`claude-core`, and keeps `agentctl` as the shared provider control plane.

## Scope
- In scope:
  - Adopt and document a unified provider architecture across codex and claude:
    - `codex-core` + `codex-cli` + `agent-provider-codex`
    - `claude-core` + `claude-cli` + `agent-provider-claude`
    - `agentctl` as provider-neutral orchestration surface.
  - Preserve existing `codex-cli` behavior and compatibility for downstream products.
  - Add a new `claude-core` crate by extracting reusable runtime logic from `agent-provider-claude`.
  - Add a new `claude-cli` crate for direct user workflows.
  - Keep full support for codex and claude in `agentctl provider|diag|workflow` surfaces.
  - Update outdated documents that conflict with this direction and mark them superseded.
- Out of scope:
  - Removing `codex-core`.
  - Forcing byte-for-byte parity for codex-only surfaces with no Claude equivalent (`agent commit`,
    `starship`) in this delivery.
  - Breaking `provider-adapter.v1` envelope semantics.
  - Breaking existing `codex-cli` JSON contract versions.

## Assumptions (if any)
1. Existing products that depend on `codex-cli` require contract-stable behavior across this work.
2. `agentctl` remains the provider-neutral surface for multi-provider orchestration.
3. Claude runtime logic currently in `agent-provider-claude` can be extracted without changing
   external provider-adapter behavior.
4. `claude-cli` can ship with environment-driven auth/config semantics first, then iterate.
5. This plan supersedes conflicting guidance in older plan documents for new implementation work.

## Success Criteria
1. Workspace contains and builds `crates/claude-core` and `crates/claude-cli`.
2. `agent-provider-claude` becomes a thin adapter over `claude-core` runtime primitives.
3. `claude-cli` exposes user-facing command groups for `agent`, `auth-state`, `diag`, and `config`
   with stable command/exit behavior.
4. `codex-cli` remains backward compatible for command surface, output behavior, and JSON contracts.
5. `agentctl` continues to support codex and claude provider selection, diagnostics, and workflow
   execution.
6. Conflicting legacy planning docs are explicitly marked superseded and point to this plan.
7. Required checks (or docs-only fast path where applicable) pass before delivery.

## Parallelization notes
- After Task 1.1 lands, Task 1.2 and Task 1.3 can run in parallel.
- After Task 2.1 lands, Task 2.2 and Task 3.1 can run in parallel.
- After Task 2.3 and Task 3.2 land, Task 4.1 and Task 4.2 can run in parallel.
- Sprint 5 documentation and rollout tasks can run in parallel with final verification prep.

## Sprint 1: Architecture decision and governance cleanup
**Goal**: Freeze the unified target architecture and remove contradictory plan guidance before implementation work.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/codex-claude-unified-cli-core-agentctl-plan.md`
  - `rg -n "Superseded by|codex-claude-unified-cli-core-agentctl-plan" docs/plans/codex-cli-claude-code-version-plan.md docs/plans/codex-core-runtime-decoupling-plan.md`
- Verify: architecture decision is explicit and conflicting legacy scope constraints are superseded.

### Task 1.1: Publish unified provider architecture contract
- **Location**:
  - `docs/specs/codex-claude-unified-architecture-v1.md`
  - `crates/codex-cli/README.md`
  - `crates/agentctl/README.md`
- **Description**: Define the canonical ownership boundary for codex and claude across
  `*-core`, `*-cli`, `agent-provider-*`, and `agentctl`, including anti-goals to prevent CLI,
  runtime, and adapter concerns from re-coupling.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Contract explicitly defines responsibilities for all four layers.
  - Contract states codex remains contract-stable while claude gains first-class CLI support.
  - Contract includes compatibility guidance for provider-neutral vs provider-specific surfaces.
- **Validation**:
  - `rg -n "ownership|codex-core|claude-core|codex-cli|claude-cli|agentctl" docs/specs/codex-claude-unified-architecture-v1.md`

### Task 1.2: Supersede conflicting legacy plan constraints
- **Location**:
  - `docs/plans/codex-cli-claude-code-version-plan.md`
  - `docs/plans/codex-core-runtime-decoupling-plan.md`
- **Description**: Add explicit supersession status blocks and forward references so prior
  out-of-scope constraints (for example, excluding `claude-cli`) are not reused for new work.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Both plans include a clear "Superseded by" banner with this plan path.
  - Both plans explicitly warn that conflicting scope boundaries are historical.
- **Validation**:
  - `rg -n "Superseded by|historical" docs/plans/codex-cli-claude-code-version-plan.md docs/plans/codex-core-runtime-decoupling-plan.md`

### Task 1.3: Freeze codex compatibility baseline before claude expansion
- **Location**:
  - `crates/codex-cli/tests/main_entrypoint.rs`
  - `crates/codex-cli/tests/dispatch.rs`
  - `crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
- **Description**: Add/confirm characterization coverage that locks codex command behavior,
  migration hints, and JSON contracts before introducing shared architectural changes.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Existing codex command and JSON behaviors are explicitly pinned by tests/spec references.
  - A compatibility checklist exists for release sign-off.
- **Validation**:
  - `cargo test -p nils-codex-cli --test main_entrypoint --test dispatch`
  - `rg -n "codex-cli\.diag\.rate-limits\.v1|codex-cli\.auth\.v1" crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`

## Sprint 2: Build claude-core and extract runtime logic
**Goal**: Create reusable Claude runtime primitives and minimize logic inside provider adapter crate.
**Demo/Validation**:
- Command(s):
  - `cargo check -p nils-claude-core`
  - `cargo test -p nils-agent-provider-claude`
- Verify: claude runtime compiles in new crate and adapter remains contract-stable.

### Task 2.1: Scaffold claude-core crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/claude-core/Cargo.toml`
  - `crates/claude-core/src/lib.rs`
  - `crates/claude-core/README.md`
  - `crates/claude-core/docs/README.md`
- **Description**: Add a new `claude-core` crate with baseline modules and docs index, then wire it
  into workspace members and dependencies.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace includes `crates/claude-core`.
  - `nils-claude-core` compiles independently.
  - README states runtime-only ownership and anti-goals.
- **Validation**:
  - `cargo check -p nils-claude-core`

### Task 2.2: Extract Claude runtime modules from provider adapter crate
- **Location**:
  - `crates/agent-provider-claude/src/config.rs`
  - `crates/agent-provider-claude/src/client.rs`
  - `crates/agent-provider-claude/src/prompts.rs`
  - `crates/claude-core/src/config.rs`
  - `crates/claude-core/src/client.rs`
  - `crates/claude-core/src/prompts.rs`
- **Description**: Move reusable config/client/prompt-intent logic into `claude-core` while keeping
  public behavior unchanged.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Runtime logic resides in `claude-core`; adapter imports core APIs.
  - Error categories/codes and retryability semantics remain stable.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test client_contract --test execute_contract`

### Task 2.3: Add claude-core contract tests and dependency boundaries
- **Location**:
  - `crates/claude-core/tests/config_contract.rs`
  - `crates/claude-core/tests/client_contract.rs`
  - `crates/claude-core/tests/prompts_contract.rs`
  - `crates/agent-provider-claude/tests/dependency_boundary.rs`
- **Description**: Add direct core crate tests and enforce boundaries so provider adapter does not
  re-accumulate runtime concerns.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Core tests cover success/failure cases equivalent to adapter behavior.
  - Boundary tests fail if adapter reintroduces duplicated runtime internals.
- **Validation**:
  - `cargo test -p nils-claude-core`
  - `cargo test -p nils-agent-provider-claude --test adapter_contract --test dependency_boundary`

## Sprint 3: Deliver user-facing claude-cli
**Goal**: Ship a first-class Claude CLI for end users while preserving explicit unsupported semantics.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-claude-cli`
  - `cargo run -p nils-claude-cli -- --help`
- Verify: claude CLI is installable, testable, and contract-documented.

### Task 3.1: Scaffold claude-cli crate, command graph, and completion stubs
- **Location**:
  - `Cargo.toml`
  - `crates/claude-cli/Cargo.toml`
  - `crates/claude-cli/src/main.rs`
  - `crates/claude-cli/src/cli.rs`
  - `completions/zsh/_claude-cli`
  - `completions/bash/claude-cli`
- **Description**: Create CLI skeleton and completion files aligned with workspace completion standards.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `claude-cli` command graph compiles and prints help.
  - Completion files exist for both zsh and bash.
- **Validation**:
  - `cargo run -p nils-claude-cli -- --help`
  - `zsh -n completions/zsh/_claude-cli`
  - `bash -n completions/bash/claude-cli`

### Task 3.2: Implement agent workflows (prompt/advice/knowledge) through claude-core
- **Location**:
  - `crates/claude-cli/src/agent.rs`
  - `crates/claude-cli/src/main.rs`
  - `crates/claude-cli/tests/agent_commands.rs`
  - `crates/claude-core/src/exec.rs`
- **Description**: Implement core user workflows matching codex intent classes but backed by
  Claude runtime execution and stable envelope/output behavior.
- **Dependencies**:
  - Task 2.3
  - Task 3.1
- **Complexity**: 8
- **Acceptance criteria**:
  - `prompt`, `advice`, and `knowledge` commands execute via core runtime.
  - Exit semantics are deterministic and tested for success/failure classes.
- **Validation**:
  - `cargo test -p nils-claude-cli --test agent_commands`

### Task 3.3: Implement claude-cli auth-state/diag/config surfaces with explicit unsupported policy
- **Location**:
  - `crates/claude-cli/src/auth.rs`
  - `crates/claude-cli/src/diag.rs`
  - `crates/claude-cli/src/config.rs`
  - `crates/claude-cli/tests/auth_diag_config.rs`
- **Description**: Provide user-visible commands for Claude auth-state and diagnostics, and define
  stable unsupported behavior for codex-only features that have no Claude equivalent.
- **Dependencies**:
  - Task 2.3
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Auth/diag/config commands have deterministic output and exit behavior.
  - Unsupported codex-only commands return stable error guidance.
- **Validation**:
  - `cargo test -p nils-claude-cli --test auth_diag_config`

### Task 3.4: Publish claude-cli contracts and migration docs
- **Location**:
  - `crates/claude-cli/README.md`
  - `crates/claude-cli/docs/specs/claude-cli-json-contract-v1.md`
  - `crates/claude-cli/docs/runbooks/codex-to-claude-cli-migration.md`
- **Description**: Document command ownership, JSON contracts, compatibility limits, and migration
  paths from codex-centric usage.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
- **Complexity**: 6
- **Acceptance criteria**:
  - CLI docs include machine-consumable contract references.
  - Migration runbook covers command mapping and unsupported alternatives.
- **Validation**:
  - `rg -n "schema_version|unsupported|migration|agent prompt|auth-state|diag" crates/claude-cli/docs/specs/claude-cli-json-contract-v1.md crates/claude-cli/docs/runbooks/codex-to-claude-cli-migration.md`

## Sprint 4: Keep agentctl as shared orchestration surface
**Goal**: Ensure provider-neutral workflows remain stable for codex and claude after new CLI/core split.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-agentctl --test provider_registry --test provider_commands --test workflow_run --test diag_capabilities --test diag_doctor`
- Verify: `agentctl` continues to orchestrate both providers with deterministic readiness and workflow behavior.

### Task 4.1: Rewire agent-provider-claude integration on top of claude-core
- **Location**:
  - `crates/agent-provider-claude/Cargo.toml`
  - `crates/agent-provider-claude/src/adapter.rs`
  - `crates/agent-provider-claude/tests/adapter_contract.rs`
- **Description**: Ensure adapter semantics remain stable while consuming `claude-core` and exposing
  provider-adapter envelopes used by `agentctl`.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Adapter continues to report `provider-adapter.v1` stable behavior.
  - No behavioral drift in mapped error categories/codes.
- **Validation**:
  - `cargo test -p nils-agent-provider-claude --test adapter_contract --test mock_contract`

### Task 4.2: Update agentctl workflow/diagnostic coverage for unified architecture
- **Location**:
  - `crates/agentctl/src/provider/registry.rs`
  - `crates/agentctl/tests/provider_commands.rs`
  - `crates/agentctl/tests/workflow_run.rs`
  - `crates/agentctl/tests/diag_capabilities.rs`
  - `crates/agentctl/tests/diag_doctor.rs`
- **Description**: Validate and, where required, update registry, selection, diagnostics, and
  workflow execution tests so codex and claude both remain first-class provider options.
- **Dependencies**:
  - Task 4.1
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Provider list/healthcheck and workflow run pass for both providers.
  - Diagnostics clearly report readiness causes for each provider.
- **Validation**:
  - `cargo test -p nils-agentctl --test provider_commands --test workflow_run --test diag_capabilities --test diag_doctor`

### Task 4.3: Align wrappers and migration hints with dual-CLI reality
- **Location**:
  - `wrappers/codex-cli`
  - `wrappers/agentctl`
  - `docs/runbooks/wrappers-mode-usage.md`
- **Description**: Keep current codex compatibility routing intact while documenting and supporting
  new claude CLI usage and agentctl orchestration expectations.
- **Dependencies**:
  - Task 3.4
  - Task 4.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Wrapper behavior remains deterministic by mode.
  - Migration hints no longer imply claude must only flow through codex-oriented paths.
- **Validation**:
  - `scripts/ci/wrapper-mode-smoke.sh`
  - `rg -n "claude-cli|agentctl|provider-neutral orchestration" docs/runbooks/wrappers-mode-usage.md`

## Sprint 5: Rollout safety, docs consolidation, and release readiness
**Goal**: Complete migration with low regression risk and clear operator guidance.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
- Verify: required checks and coverage gate pass with unified architecture and updated docs.

### Task 5.1: Codex compatibility regression gate
- **Location**:
  - `crates/codex-cli/tests/main_entrypoint.rs`
  - `crates/codex-cli/tests/dispatch.rs`
  - `crates/agent-provider-codex/tests/adapter_contract.rs`
- **Description**: Re-run and extend codex-centric regression tests to guarantee existing consumers
  are unaffected by claude architecture additions.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Codex command behavior and JSON contract snapshots remain stable.
  - No new codex-specific regressions are introduced.
- **Validation**:
  - `cargo test -p nils-codex-cli`
  - `cargo test -p nils-agent-provider-codex`

### Task 5.2: Cross-provider operator runbook updates
- **Location**:
  - `crates/agentctl/docs/runbooks/codex-to-claude-mapping.md`
  - `crates/agentctl/README.md`
  - `crates/codex-cli/README.md`
  - `crates/claude-cli/README.md`
- **Description**: Update operator docs so command routing, ownership boundaries, and migration
  guidance reflect codex-cli + claude-cli + agentctl coexistence.
- **Dependencies**:
  - Task 3.4
  - Task 4.3
- **Complexity**: 5
- **Acceptance criteria**:
  - Documentation is consistent on when to use each CLI vs agentctl.
  - No remaining references claim claude-cli is out of scope.
- **Validation**:
  - `rg -n "out of scope|claude-cli|agentctl|codex-cli" crates/agentctl/docs/runbooks/codex-to-claude-mapping.md crates/agentctl/README.md crates/codex-cli/README.md crates/claude-cli/README.md`

### Task 5.3: Final required checks and release cutover checklist
- **Location**:
  - `DEVELOPMENT.md`
  - `docs/plans/codex-claude-unified-cli-core-agentctl-plan.md`
  - `docs/runbooks/codex-claude-dual-cli-rollout.md`
  - `release/crates-io-publish-order.txt`
- **Description**: Execute workspace required checks, coverage gate, and release readiness checklist
  for dual-CLI support, including rollback checkpoints.
- **Dependencies**:
  - Task 5.1
  - Task 5.2
- **Complexity**: 7
- **Acceptance criteria**:
  - All required checks pass.
  - Coverage gate remains >= 85.
  - Dual-CLI rollout checklist exists with explicit gating and fallback steps.
  - Rollback checkpoints are documented and tested.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - `rg -n \"gating|fallback|rollback|agentctl|codex-cli|claude-cli\" docs/runbooks/codex-claude-dual-cli-rollout.md`

## Testing Strategy
- Unit:
  - Add dedicated unit/contract tests in `claude-core` for config parsing, prompt intent rendering,
    HTTP error mapping, timeout/retry behavior, and redaction policies.
- Integration:
  - Add crate-level integration tests for `claude-cli`, `agent-provider-claude`, and `agentctl`
    provider/workflow diagnostics to verify end-to-end behavior and stable envelopes.
- E2E/manual:
  - Validate local CLI flows with configured and missing `ANTHROPIC_API_KEY`, including
    deterministic failure messages and exit codes.
  - Validate wrapper behavior in `auto|debug|installed` modes for codex-cli and agentctl routing.

## Risks & gotchas
- Codex compatibility regression risk while introducing shared architectural rules.
- Claude runtime extraction risk if adapter and CLI consume subtly different defaults.
- Scope creep risk from trying to force unsupported codex-only features into claude-cli.
- Documentation drift risk if old plans remain active without explicit supersession markers.

## Rollback plan
- If `claude-core` extraction causes regressions:
  - Temporarily re-point `agent-provider-claude` to previous internal modules behind a revert commit.
  - Keep `claude-core` crate present but non-authoritative until parity gaps are closed.
- If `claude-cli` quality gate fails:
  - Keep `agentctl` + `agent-provider-claude` as supported path and defer `claude-cli` release cut.
  - Leave migration docs marked as preview with explicit status.
- If codex compatibility regresses:
  - Revert codex-facing changes first, keep claude work isolated on branch-level commits.
  - Re-run required checks and compatibility regression suite before re-attempt.
- If docs diverge again:
  - Re-apply supersession markers and link checks in CI docs audit before next merge.
