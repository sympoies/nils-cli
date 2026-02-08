# Plan: Multi-provider agent architecture with separated `codex-cli` and `agentctl` responsibilities

## Overview
This plan redesigns the current direction so `codex-cli` remains strictly OpenAI/Codex-focused, while a new provider-neutral `agentctl` owns cross-provider local automation, diagnostics, orchestration, and debug workflows. The architecture must support future `claude` and `gemini` CLI providers without rewriting control-plane logic. Existing automation CLIs (`macos-agent`, `screen-record`, `image-processing`, `fzf-cli`) are integrated through stable machine-readable contracts consumed by `agentctl`, not by provider-specific CLIs.

## Scope
- In scope:
  - Define and implement clear role boundaries between `codex-cli` and `agentctl`.
  - Introduce provider-neutral runtime interfaces and shared schemas for multi-provider execution.
  - Move control-plane capabilities (doctor, debug bundle, workflow runner) to `agentctl`.
  - Keep `codex-cli` focused on OpenAI/Codex-specific concerns (auth, provider diagnostics, provider execution policies).
  - Integrate `macos-agent`, `screen-record`, `image-processing`, and `fzf-cli` via `agentctl` adapters.
  - Add provider onboarding path for future `claude` and `gemini` implementations.
- Out of scope:
  - Implementing full production-grade `claude` or `gemini` providers in this phase.
  - Breaking existing `codex-cli` user workflows without compatibility shims.
  - Adding remote orchestration services.

## Assumptions (if any)
1. `codex-cli` should remain provider-specific by design, not a generic multi-provider orchestrator.
2. `agentctl` can be added as a new workspace crate and primary control-plane entrypoint.
3. Provider-specific CLIs can expose/consume shared runtime contracts without leaking provider internals.
4. Desktop automation checks require environment-aware fallbacks (stub/test-mode in CI).
5. Artifact outputs can be written under `${CODEX_HOME}/out` with deterministic structure.

## Sprint 1: Architecture boundary and shared runtime contract
**Goal**: Lock the separation model and create a reusable provider-neutral foundation.
**Demo/Validation**:
- Command(s):
  - `cargo run -p plan-tooling -- validate --file docs/plans/codex-local-dev-debug-automation-upgrade-plan.md`
  - `cargo test -p codex-cli -- main_entrypoint`
  - `cargo test -p agent-runtime-core`
- Verify:
  - Responsibility boundaries are explicit and testable.
  - Shared runtime crate compiles and exposes provider adapter interfaces.

**Parallelization notes**:
- `Task 1.1` should run first.
- `Task 1.2` and `Task 1.3` can run in parallel after `Task 1.1`.

### Task 1.1: Write boundary spec and migration ADR
- **Location**:
  - `docs/adr/adr-agentctl-provider-boundary.md`
  - `crates/codex-cli/README.md`
  - `README.md`
- **Description**: Define strict role boundaries: `codex-cli` handles provider-specific OpenAI/Codex operations only; `agentctl` handles provider-neutral orchestration and local automation integration. Document migration principles, ownership boundaries, and compatibility policy.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - ADR contains explicit allow/deny lists for each CLI scope.
  - README docs show command ownership split with migration notes.
  - No contradictory ownership statements remain in docs.
- **Validation**:
  - `rg -n "codex-cli|agentctl|provider-specific|provider-neutral" docs/adr/adr-agentctl-provider-boundary.md README.md crates/codex-cli/README.md`

### Task 1.2: Introduce provider-neutral core crate (`agent-runtime-core`)
- **Location**:
  - `crates/agent-runtime-core/Cargo.toml`
  - `crates/agent-runtime-core/src/lib.rs`
  - `crates/agent-runtime-core/src/provider.rs`
  - `crates/agent-runtime-core/src/schema.rs`
  - `crates/agent-runtime-core/tests/provider_contract.rs`
  - `Cargo.toml`
- **Description**: Add a shared crate with stable traits and schemas for provider adapters (`capabilities`, `healthcheck`, `execute`, `limits`, `auth-state`) plus normalized error/result envelopes used by `agentctl`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Provider trait and schema are versioned and documented.
  - Contract tests validate envelope compatibility and error categorization.
  - Crate is provider-neutral and contains no OpenAI-specific logic.
- **Validation**:
  - `cargo test -p agent-runtime-core`
  - `cargo clippy -p agent-runtime-core --all-targets -- -D warnings`

### Task 1.3: Scaffold `agentctl` crate and top-level command groups
- **Location**:
  - `crates/agentctl/Cargo.toml`
  - `crates/agentctl/src/main.rs`
  - `crates/agentctl/src/cli.rs`
  - `crates/agentctl/src/lib.rs`
  - `crates/agentctl/tests/dispatch.rs`
  - `Cargo.toml`
- **Description**: Add a new CLI crate and define initial group structure (`provider`, `diag`, `debug`, `workflow`, `automation`) with help/exit behavior and compatibility with workspace standards.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - `agentctl --help` shows provider-neutral command groups.
  - Dispatch tests cover help and unknown-command behavior.
  - Workspace builds with new crate membership.
- **Validation**:
  - `cargo run -p agentctl -- --help`
  - `cargo test -p agentctl -- dispatch`

## Sprint 2: Keep `codex-cli` provider-specific and add Codex adapter
**Goal**: Refactor `codex-cli` to explicit provider scope and expose a reusable Codex adapter for `agentctl`.
**Demo/Validation**:
- Command(s):
  - `cargo test -p codex-cli`
  - `cargo test -p agent-provider-codex`
  - `cargo run -p codex-cli -- --help`
- Verify:
  - `codex-cli` excludes generic orchestration concerns.
  - Codex provider adapter can be called by `agentctl` through shared trait.

**Parallelization notes**:
- `Task 2.1` should start first.
- `Task 2.2` starts after `Task 2.1` interface freeze.
- `Task 2.3` depends on `Task 2.1` and `Task 2.2`.

### Task 2.1: Enforce `codex-cli` command-scope boundary
- **Location**:
  - `crates/codex-cli/src/cli.rs`
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/tests/main_entrypoint.rs`
  - `crates/codex-cli/README.md`
- **Description**: Audit and enforce that `codex-cli` only contains OpenAI/Codex-specific operations (auth, provider diagnostics, provider execution wrappers, provider prompt tooling). Explicitly disallow generic multi-provider orchestration commands in this crate.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Command tree reflects provider-specific scope only.
  - Tests guard against accidental reintroduction of provider-neutral groups.
  - README states codex-only contract clearly.
- **Validation**:
  - `cargo test -p codex-cli -- main_entrypoint`
  - `cargo run -p codex-cli -- --help | rg -n "agent|auth|diag|config|starship"`

### Task 2.2: Add `agent-provider-codex` adapter crate
- **Location**:
  - `crates/agent-provider-codex/Cargo.toml`
  - `crates/agent-provider-codex/src/lib.rs`
  - `crates/agent-provider-codex/src/adapter.rs`
  - `crates/agent-provider-codex/tests/adapter_contract.rs`
  - `Cargo.toml`
- **Description**: Implement Codex adapter over existing `codex-cli` internals to satisfy `agent-runtime-core` provider trait and expose normalized capability/health/execute surfaces for `agentctl`.
- **Dependencies**:
  - Task 1.2
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Adapter returns normalized provider metadata and errors.
  - Adapter tests cover unavailable-binary and disabled-policy cases.
  - Adapter does not duplicate business logic unnecessarily.
- **Validation**:
  - `cargo test -p agent-provider-codex`
  - `cargo clippy -p agent-provider-codex --all-targets -- -D warnings`

### Task 2.3: Add compatibility shims and migration messaging
- **Location**:
  - `wrappers/codex-cli`
  - `completions/zsh/_codex-cli`
  - `completions/bash/codex-cli`
  - `README.md`
  - `crates/codex-cli/README.md`
- **Description**: Keep existing codex workflows stable while introducing `agentctl`. Add migration hints that direct provider-neutral use cases to `agentctl` and keep codex-specific tasks in `codex-cli`.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Existing codex wrapper/completion behavior remains functional.
  - Migration hints are concise and consistent in docs/help text.
  - No breaking change for current codex-centric users.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `cargo run -p codex-cli -- --help`
  - `rg -n "agentctl|migration|provider-neutral" README.md crates/codex-cli/README.md`

## Sprint 3: Implement `agentctl` control plane and automation integrations
**Goal**: Deliver provider-neutral control-plane features with local automation integration.
**Demo/Validation**:
- Command(s):
  - `cargo test -p agentctl -- diag_`
  - `cargo test -p agentctl -- debug_bundle_`
  - `cargo test -p agentctl -- workflow_`
  - `cargo run -p agentctl -- diag doctor --format json`
- Verify:
  - `agentctl` can diagnose environment/provider readiness.
  - `agentctl` can produce multi-tool debug bundles and run declarative workflows.

**Parallelization notes**:
- `Task 3.1` should start first.
- `Task 3.2` depends on `Task 3.1`.
- `Task 3.3` and `Task 3.4` can run in parallel after `Task 3.2` interface stabilization.
- `Task 3.5` depends on `Task 3.3` and `Task 3.4`.

### Task 3.1: Implement `agentctl provider` registry and selection
- **Location**:
  - `crates/agentctl/src/cli.rs`
  - `crates/agentctl/src/provider/mod.rs`
  - `crates/agentctl/src/provider/registry.rs`
  - `crates/agentctl/src/provider/commands.rs`
  - `crates/agentctl/tests/provider_registry.rs`
- **Description**: Build provider registration and selection flow in `agentctl` using `agent-runtime-core` adapters. Include provider listing, default-provider selection, and healthcheck execution.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - `agentctl provider list` returns registered providers with status.
  - Default provider resolution is deterministic and overrideable.
  - Registry behavior is fully covered by tests.
- **Validation**:
  - `cargo test -p agentctl -- provider_registry`
  - `cargo run -p agentctl -- provider list --format json`

### Task 3.2: Implement `agentctl diag doctor` and `diag capabilities`
- **Location**:
  - `crates/agentctl/src/diag/mod.rs`
  - `crates/agentctl/src/diag/doctor.rs`
  - `crates/agentctl/src/diag/capabilities.rs`
  - `crates/agentctl/tests/diag_doctor.rs`
  - `crates/agentctl/tests/diag_capabilities.rs`
- **Description**: Add provider-neutral diagnostics and capability inventory, including provider checks plus automation tool readiness (`macos-agent`, `screen-record`, `image-processing`, `fzf-cli`). Include stub/test-mode strategy for CI portability.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Diagnostics include provider and automation readiness in a single normalized contract.
  - Probe mode supports deterministic test-mode execution in CI.
  - Failure hints distinguish missing dependency, permission, and platform limitation.
- **Validation**:
  - `CODEX_MACOS_AGENT_TEST_MODE=1 CODEX_SCREEN_RECORD_TEST_MODE=1 cargo test -p agentctl -- diag_`
  - `cargo run -p agentctl -- diag doctor --format json`

### Task 3.3: Implement `agentctl debug bundle` with automation adapters
- **Location**:
  - `crates/agentctl/src/debug/mod.rs`
  - `crates/agentctl/src/debug/bundle.rs`
  - `crates/agentctl/src/debug/schema.rs`
  - `crates/agentctl/src/debug/sources/macos_agent.rs`
  - `crates/agentctl/src/debug/sources/screen_record.rs`
  - `crates/agentctl/src/debug/sources/image_processing.rs`
  - `crates/agentctl/src/debug/sources/git_context.rs`
  - `crates/agentctl/tests/debug_bundle.rs`
- **Description**: Implement one-shot bundle collection with manifest index, partial-failure handling, and artifact normalization hooks. Integrate git context and automation artifacts while preserving deterministic output layout.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 9
- **Acceptance criteria**:
  - Bundle manifest is versioned and always emitted.
  - Partial failures are visible without losing successful artifact references.
  - Artifact paths are deterministic under `${CODEX_HOME}/out` or configured output dir.
- **Validation**:
  - `cargo test -p agentctl -- debug_bundle_`
  - `cargo run -p agentctl -- debug bundle --output-dir ${CODEX_HOME:-$HOME/.codex}/out/agentctl-debug-demo`

### Task 3.4: Implement `agentctl workflow run` declarative orchestration
- **Location**:
  - `crates/agentctl/src/workflow/mod.rs`
  - `crates/agentctl/src/workflow/schema.rs`
  - `crates/agentctl/src/workflow/run.rs`
  - `crates/agentctl/tests/workflow_run.rs`
  - `crates/agentctl/tests/fixtures/workflow/minimal.json`
- **Description**: Add provider-neutral workflow runner with step retries/timeouts, provider-targeted execution steps, and automation steps invoking supported CLIs. Emit structured step ledger and final run summary.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 9
- **Acceptance criteria**:
  - Workflow schema supports provider and automation steps in one file.
  - Runner supports fail-fast and continue-on-error modes.
  - Step ledger captures stdout/stderr/exit code/elapsed time per step.
- **Validation**:
  - `cargo test -p agentctl -- workflow_run`
  - `cargo run -p agentctl -- workflow run --file crates/agentctl/tests/fixtures/workflow/minimal.json`

### Task 3.5: Add workflow automation-step adapters and cross-tool execution tests
- **Location**:
  - `crates/agentctl/src/workflow/run.rs`
  - `crates/agentctl/src/workflow/steps/automation.rs`
  - `crates/agentctl/src/workflow/steps/macos_agent.rs`
  - `crates/agentctl/src/workflow/steps/screen_record.rs`
  - `crates/agentctl/src/workflow/steps/image_processing.rs`
  - `crates/agentctl/src/workflow/steps/fzf_cli.rs`
  - `crates/agentctl/tests/workflow_automation_steps.rs`
  - `crates/agentctl/tests/fixtures/workflow/automation-mixed.json`
- **Description**: Implement explicit workflow step adapters for `macos-agent`, `screen-record`, `image-processing`, and `fzf-cli`, including normalized argument mapping, step-level log capture, and deterministic stub/test-mode pathways for CI. Ensure these adapters are first-class workflow primitives rather than undocumented generic shell shortcuts.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 8
- **Acceptance criteria**:
  - Workflow manifests can invoke all four automation CLIs through typed step definitions.
  - Step result envelopes include normalized command provenance and artifact pointers.
  - CI-friendly tests pass with deterministic test/stub modes for desktop-sensitive tools.
- **Validation**:
  - `CODEX_MACOS_AGENT_TEST_MODE=1 CODEX_SCREEN_RECORD_TEST_MODE=1 cargo test -p agentctl -- workflow_automation_steps`
  - `cargo run -p agentctl -- workflow run --file crates/agentctl/tests/fixtures/workflow/automation-mixed.json`

## Sprint 4: Future-provider extensibility (`claude` / `gemini`) and release hardening
**Goal**: Prove extensibility path and complete delivery-quality gates.
**Demo/Validation**:
- Command(s):
  - `cargo test -p agentctl -p agent-runtime-core -p agent-provider-codex`
  - `cargo test -p codex-cli`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - Provider onboarding contract is executable for new providers.
  - Repo checks pass after architecture split.

**Parallelization notes**:
- `Task 4.1` and `Task 4.2` can run in parallel.
- `Task 4.3` depends on `Task 4.1` and `Task 4.2`.

### Task 4.1: Add provider onboarding template and stub adapters for `claude` and `gemini`
- **Location**:
  - `crates/agent-runtime-core/src/provider.rs`
  - `crates/agentctl/src/provider/registry.rs`
  - `docs/runbooks/provider-onboarding.md`
  - `crates/agent-provider-claude/Cargo.toml`
  - `crates/agent-provider-claude/src/lib.rs`
  - `crates/agent-provider-gemini/Cargo.toml`
  - `crates/agent-provider-gemini/src/lib.rs`
  - `Cargo.toml`
- **Description**: Add onboarding runbook and compile-only provider stubs for `claude` and `gemini` that implement trait skeletons and registry wiring without full auth/execute behavior yet.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Registry can list codex + stub providers with clear maturity status.
  - Onboarding runbook includes minimum required files, tests, and validation checklist.
  - Stub providers compile and pass contract skeleton tests.
- **Validation**:
  - `cargo test -p agent-provider-claude -p agent-provider-gemini`
  - `cargo run -p agentctl -- provider list --format json`

### Task 4.2: Docs and completion updates for split architecture
- **Location**:
  - `README.md`
  - `crates/agentctl/README.md`
  - `crates/codex-cli/README.md`
  - `completions/zsh/_agentctl`
  - `completions/bash/agentctl`
  - `tests/zsh/completion.test.zsh`
  - `BINARY_DEPENDENCIES.md`
- **Description**: Document command ownership split (`codex-cli` vs `agentctl`), provider-neutral workflow examples, and future-provider onboarding path. Add shell completions for `agentctl`.
- **Dependencies**:
  - Task 1.1
  - Task 2.3
  - Task 4.1
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Docs provide explicit “use this CLI for this job” matrix.
  - `agentctl` completion works in existing zsh completion test flow.
  - Dependency doc includes new crates and runtime requirements.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "agentctl|codex-cli|provider-neutral|provider-specific|claude|gemini" README.md crates/agentctl/README.md crates/codex-cli/README.md`

### Task 4.3: End-to-end validation and rollout checklist
- **Location**:
  - `crates/agentctl/tests/workflow_run.rs`
  - `crates/agentctl/tests/debug_bundle.rs`
  - `crates/codex-cli/tests/main_entrypoint.rs`
  - `docs/plans/codex-local-dev-debug-automation-upgrade-plan.md`
- **Description**: Run mandatory checks, targeted crate suites, and representative control-plane smoke scenarios. Produce concise rollout checklist and release confidence summary.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Mandatory repository checks pass.
  - Representative split-flow smoke tests pass (`codex-cli` provider ops + `agentctl` orchestration ops).
  - Plan remains valid under `plan-tooling`.
- **Validation**:
  - `plan-tooling validate --file docs/plans/codex-local-dev-debug-automation-upgrade-plan.md`
  - `cargo test -p agentctl -p agent-runtime-core -p agent-provider-codex -p codex-cli`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit:
  - Provider trait/schema validation in `agent-runtime-core`.
  - CLI parse/dispatch and policy checks in `codex-cli` and `agentctl`.
- Integration:
  - Adapter contract tests (`agent-provider-codex`, future stub providers).
  - `agentctl` diagnostic/debug/workflow flows with stubbed external commands.
- E2E/manual:
  - Local macOS smoke for automation integrations (`macos-agent`, `screen-record`) under explicit permissions.
  - Multi-step workflow replay using provider + automation mixed manifests.
- Regression guard:
  - Preserve `codex-cli` behavior for codex-specific workflows.
  - Require compatibility tests for migration shims during split transition.

## Risks & gotchas
- Boundary drift may re-couple provider logic and control-plane logic over time.
- Cross-crate schema/version mismatch can break adapters unexpectedly.
- Desktop permission and platform differences can cause flaky probe behavior without strict test-mode discipline.
- Completion/wrapper drift can confuse users during migration.

## Rollback plan
- Keep split changes additive and feature-flagged where possible during rollout.
- If `agentctl` rollout is unstable, keep `codex-cli` provider flows as primary path and disable affected `agentctl` groups temporarily.
- Revert individual adapter crates (`agent-provider-*`) independently without reverting core provider-specific functionality.
- Retain migration shims/docs for previous command paths until two release cycles after stabilization.

## Sprint 4 rollout checklist (2026-02-08)
- [x] Added provider onboarding runbook: `docs/runbooks/provider-onboarding.md`.
- [x] Added compile-only provider stubs: `agent-provider-claude` and `agent-provider-gemini`.
- [x] Registered built-in provider set in `agentctl` registry (`codex`, `claude`, `gemini`) with maturity metadata.
- [x] Updated docs + ownership matrix for split architecture (`README.md`, `crates/agentctl/README.md`, `crates/codex-cli/README.md`).
- [x] Added `agentctl` shell completions (`completions/zsh/_agentctl`, `completions/bash/agentctl`) and wired completion tests.
- [x] Updated dependency inventory with provider maturity/runtime expectations (`BINARY_DEPENDENCIES.md`).
- [x] Validation completed:
  - `plan-tooling validate --file docs/plans/codex-local-dev-debug-automation-upgrade-plan.md`
  - `cargo test -p agentctl -p agent-runtime-core -p agent-provider-codex -p codex-cli`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- [x] Coverage target met:
  - `cargo llvm-cov --workspace --summary-only`
  - Workspace line coverage: `85.04%` (threshold `>=85%`).

### Sprint 4 release confidence summary
- Provider-neutral split is stable: `codex-cli` remains provider-specific; `agentctl` owns orchestration groups.
- Built-in provider registry now proves future-provider extensibility with explicit maturity signaling.
- Full repository gates and completion tests passed in this rollout check.
