# Plan: Gemini 1:1 Codex Architecture Parity

## Overview
This plan upgrades the current `gemini` lane from stub to full implementation by mirroring the
Codex architecture split: `gemini-core` for runtime primitives, `gemini-cli` for provider-specific
UX/contracts, and `agent-provider-gemini` for `provider-adapter.v1` mapping. The rollout is
additive and must not regress existing Codex or Claude behavior. The target outcome is
contract-stable Gemini support with publish-ready crates, completion assets, and full workspace
validation.

## Scope
- In scope: new `gemini-core` and `gemini-cli` crates, `agent-provider-gemini` promotion from
  `stub` to `stable`, `agentctl` registry/health contract updates, docs/contracts/completions, and
  required checks including coverage gate.
- Out of scope: redesigning `agentctl` orchestration model, removing existing Codex/Claude
  surfaces, or introducing non-Rust runtime daemons.

## Assumptions (if any)
1. `1:1` means architectural and contract parity with the Codex lane, while provider identifiers,
   environment names, and wire specifics use Gemini naming.
2. Gemini execution is performed through a deterministic runtime surface that can be wrapped by
   `gemini-core` (CLI binary and/or HTTP transport), and that surface is available in local/CI.
3. Existing Codex JSON contracts (`codex-cli.diag.rate-limits.v1`, `codex-cli.auth.v1`) remain
   unchanged; Gemini gets separate schema identifiers.
4. New crates are publishable and should be wired into release order unless release owners decide
   otherwise during final cutover review.

## Success Criteria
- `agent-provider-gemini` reports `maturity=stable` and passes deterministic adapter contract tests.
- `gemini-cli` exposes Codex-parity command topology and JSON envelope guarantees for supported
  diag/auth surfaces.
- `gemini-core` becomes the only runtime dependency for `gemini-cli` and `agent-provider-gemini`
  (no provider-to-CLI coupling).
- Workspace required checks pass, including completion checks and line coverage gate (`>= 85.00%`).

## Dependency and Parallelization Plan
- Track A (foundational): Task 1.1 -> Task 1.2 and Task 1.5 -> Task 2.1.
- Track B (CLI shell): Task 1.3 starts after Task 1.2; Task 3.2 and Task 3.5 run in parallel after
  Task 3.1; Task 3.6 converges Sprint 3 outputs.
- Track C (provider): Task 4.1 starts after Task 2.1 and Task 1.5; Task 4.2 and Task 4.4 run in
  parallel after Task 4.1.
- Integration convergence: Task 1.4, Task 3.6, Task 4.3, and Task 5.1 must complete before
  Task 5.2.

## Sprint 1: Contract Freeze and Workspace Scaffolding
**Goal**: lock architecture/contracts first, then create publish-ready crate shells and baseline
workspace wiring.
**Demo/Validation**:
- Command(s):
  - `cargo metadata --no-deps --format-version 1 | rg "nils-gemini-core|nils-gemini-cli"`
  - `cargo run -p nils-gemini-cli -- --help`
  - `plan-tooling validate --file docs/plans/gemini-codex-1-to-1-parity-plan.md`
- Verify:
  - New crates are discoverable in workspace metadata and have runnable CLI entrypoint.

### Task 1.1: Define Gemini parity contract and ownership boundaries
- **Location**:
  - `docs/specs/codex-gemini-unified-architecture-v1.md`
  - `crates/agent-provider-gemini/docs/specs/codex-cli-gemini-parity-matrix-v1.md`
  - `docs/runbooks/codex-gemini-dual-cli-rollout.md`
- **Description**: Write canonical architecture/spec docs that map Codex lane responsibilities to
  Gemini lane responsibilities, including parity class (`exact`, `semantic`, `unsupported`) per
  command surface and rollout constraints.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Ownership boundaries for `gemini-core`, `gemini-cli`, `agent-provider-gemini`, and `agentctl`
    are explicit and non-overlapping.
  - Parity matrix lists every Codex user-facing surface and Gemini mapping with deterministic
    behavior notes.
  - Rollout doc includes cutover gates, fallback, and rollback checkpoints.
- **Validation**:
  - `rg -n "gemini-core|gemini-cli|agent-provider-gemini|agentctl" docs/specs/codex-gemini-unified-architecture-v1.md`
  - `rg -n "exact|semantic|unsupported" crates/agent-provider-gemini/docs/specs/codex-cli-gemini-parity-matrix-v1.md`
  - `rg -n "cutover|fallback|rollback" docs/runbooks/codex-gemini-dual-cli-rollout.md`

### Task 1.2: Scaffold publish-ready gemini-core crate
- **Location**:
  - `Cargo.toml`
  - `crates/gemini-core/Cargo.toml`
  - `crates/gemini-core/src/lib.rs`
  - `crates/gemini-core/README.md`
  - `crates/gemini-core/docs/README.md`
- **Description**: Create `gemini-core` crate skeleton with workspace-standard package metadata,
  docs index, and module exports mirroring `codex-core` boundary shape.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace includes `crates/gemini-core` as a member.
  - Package metadata follows repository conventions (name, version, edition, license, repository).
  - Crate docs index exists so docs placement audit passes.
- **Validation**:
  - `cargo metadata --no-deps --format-version 1 | rg "nils-gemini-core"`
  - `test -f crates/gemini-core/src/lib.rs`
  - `test -f crates/gemini-core/docs/README.md`

### Task 1.3: Scaffold publish-ready gemini-cli crate with command shell
- **Location**:
  - `Cargo.toml`
  - `crates/gemini-cli/Cargo.toml`
  - `crates/gemini-cli/src/lib.rs`
  - `crates/gemini-cli/src/main.rs`
  - `crates/gemini-cli/src/cli.rs`
  - `crates/gemini-cli/README.md`
  - `crates/gemini-cli/docs/README.md`
- **Description**: Create `gemini-cli` crate shell with clap parser, `-V/--version` support,
  top-level command groups matching Codex topology, and completion export subcommand placeholder.
- **Dependencies**:
  - Task 1.2
  - Task 1.5
- **Complexity**: 5
- **Acceptance criteria**:
  - Binary target `gemini-cli` is registered and executable.
  - Help output lists parity command groups (`agent`, `auth`, `diag`, `config`, `starship`,
    `completion`).
  - CLI returns usage exit semantics aligned with workspace rules.
- **Validation**:
  - `cargo metadata --no-deps --format-version 1 | rg "nils-gemini-cli"`
  - `cargo run -p nils-gemini-cli -- --help`
  - `cargo run -p nils-gemini-cli -- -V`

### Task 1.4: Add baseline release and inventory wiring for Gemini crates
- **Location**:
  - `release/crates-io-publish-order.txt`
  - `docs/reports/completion-coverage-matrix.md`
- **Description**: Wire new Gemini crates into release and completion inventory artifacts so publish
  sequencing and completion obligations are explicit from the first implementation checkpoint.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Publish order includes `nils-gemini-core`, `nils-gemini-cli`, and
    `nils-agent-provider-gemini` in dependency-safe sequence.
  - Completion coverage matrix includes `gemini-cli` with required metadata tuple.
- **Validation**:
  - `awk '/nils-gemini-core/{core=NR}/nils-gemini-cli/{cli=NR}/nils-agent-provider-gemini/{provider=NR} END{exit !(core>0 && cli>0 && provider>0 && core<cli && core<provider)}' release/crates-io-publish-order.txt`
  - `rg -n "\`gemini-cli\`" docs/reports/completion-coverage-matrix.md`
  - `rg -n "completion_mode=clap-first; completion_mode_toggles=forbidden; alternate_completion_dispatch=forbidden; generated_load_failure=fail-closed" docs/reports/completion-coverage-matrix.md`

### Task 1.5: Establish runtime viability gate and deterministic fixture strategy
- **Location**:
  - `BINARY_DEPENDENCIES.md`
  - `crates/gemini-core/README.md`
  - `crates/agent-provider-gemini/tests/fixtures/README.md`
  - `crates/agent-provider-gemini/docs/runbooks/verification-oracles.md`
- **Description**: Confirm and document the concrete Gemini runtime path, CI determinism
  requirements, and fallback behavior so downstream implementation does not stall on hidden runtime
  assumptions.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Runtime requirement for Gemini execution is documented with deterministic local and CI
    expectations.
  - Fixture/oracle policy specifies reproducible inputs and redaction constraints.
  - Unsupported fallback behavior is defined for runtime capabilities that cannot be implemented
    safely.
- **Validation**:
  - `rg -n "agent-provider-gemini|runtime requirement|stable" BINARY_DEPENDENCIES.md`
  - `rg -n "deterministic|fixture|redaction|fallback|unsupported" crates/agent-provider-gemini/tests/fixtures/README.md`

## Sprint 2: Gemini Runtime Core Parity
**Goal**: implement `gemini-core` runtime primitives and enforce dependency boundaries.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-gemini-core`
  - `bash scripts/ci/gemini-core-boundary-check.sh`
- Verify:
  - Runtime primitives are reusable by CLI and provider crates without cross-coupling.

### Task 2.1: Port Codex runtime modules into gemini-core with Gemini namespace
- **Location**:
  - `crates/gemini-core/src/auth.rs`
  - `crates/gemini-core/src/config.rs`
  - `crates/gemini-core/src/error.rs`
  - `crates/gemini-core/src/exec.rs`
  - `crates/gemini-core/src/json.rs`
  - `crates/gemini-core/src/jwt.rs`
  - `crates/gemini-core/src/paths.rs`
- **Description**: Port `codex-core` runtime capabilities into `gemini-core`, including auth
  parsing, path resolution, config snapshot, execution policy gate, and typed error mapping with
  Gemini-specific env/binary naming.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Module ownership boundary matches Codex core boundary shape.
  - Runtime helpers expose deterministic behavior for missing env, missing binary, and invalid auth
    payloads.
  - No clap/CLI UX logic is introduced in `gemini-core`.
- **Validation**:
  - `cargo test -p nils-gemini-core --lib`
  - `rg -n "pub mod auth|pub mod config|pub mod exec|pub mod paths" crates/gemini-core/src/lib.rs`
  - `if rg -n "clap::" crates/gemini-core/src; then exit 1; else exit 0; fi`

### Task 2.2: Add characterization tests for gemini-core runtime contracts
- **Location**:
  - `crates/gemini-core/tests/auth_contract.rs`
  - `crates/gemini-core/tests/paths_config_contract.rs`
  - `crates/gemini-core/tests/exec_contract.rs`
  - `crates/gemini-core/tests/error_contract.rs`
- **Description**: Port Codex core contract tests to Gemini equivalents so runtime behavior stays
  deterministic and future refactors preserve envelopes and edge-case handling.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Contract tests cover success and failure paths for auth parsing, execution policy, and path
    resolution.
  - Test fixtures avoid external network state and remain reproducible in CI.
  - Error category/code expectations are stable and documented by tests.
- **Validation**:
  - `cargo test -p nils-gemini-core --test auth_contract --test paths_config_contract --test exec_contract --test error_contract`

### Task 2.3: Enforce gemini-core dependency boundary and migration guidance
- **Location**:
  - `crates/gemini-core/docs/specs/gemini-core-boundary-v1.md`
  - `crates/gemini-core/docs/runbooks/gemini-core-migration.md`
  - `scripts/ci/gemini-core-boundary-check.sh`
  - `crates/agent-provider-gemini/tests/dependency_boundary.rs`
- **Description**: Add boundary spec/runbook and CI check that prevent `agent-provider-gemini` from
  importing `gemini-cli` internals and keep dependency direction aligned with architecture.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Boundary spec defines allowed dependency edges and anti-goals.
  - CI boundary script fails on forbidden imports/dependencies.
  - Provider dependency-boundary test is present and passing.
- **Validation**:
  - `bash scripts/ci/gemini-core-boundary-check.sh`
  - `cargo test -p nils-agent-provider-gemini --test dependency_boundary`

## Sprint 3: Gemini CLI 1:1 Command and Contract Parity
**Goal**: deliver `gemini-cli` command UX and JSON contracts mirroring Codex behavior.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-gemini-cli --test main_entrypoint --test dispatch`
  - `cargo test -p nils-gemini-cli --test completion_contract --test completion_smoke`
  - `cargo run -p nils-gemini-cli -- completion zsh | rg -- "--help|--version|--format"`
- Verify:
  - CLI command topology, exit semantics, JSON envelopes, and completion export are stable.

### Task 3.1: Port codex-cli command graph and dispatch to gemini-cli
- **Location**:
  - `crates/gemini-cli/src/main.rs`
  - `crates/gemini-cli/src/cli.rs`
  - `crates/gemini-cli/src/lib.rs`
  - `crates/gemini-cli/src/completion/mod.rs`
- **Description**: Implement clap command graph and dispatch structure in `gemini-cli` to mirror
  `codex-cli` command family layout, usage errors, help/version behavior, and legacy redirect
  handling.
- **Dependencies**:
  - Task 1.3
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Command groups and subcommands align with parity matrix commitments.
  - Exit code behavior matches workspace standards for success, usage, and runtime errors.
  - Completion export path exists and is wired to clap generation.
- **Validation**:
  - `cargo test -p nils-gemini-cli --test main_entrypoint --test dispatch`
  - `cargo run -p nils-gemini-cli -- help`
- `cargo run -p nils-gemini-cli -- unknown-command >/dev/null 2>&1; test $? -eq 64`

### Task 3.2: Port shared CLI utility modules with codex parity behavior
- **Location**:
  - `crates/gemini-cli/src/fs.rs`
  - `crates/gemini-cli/src/diag_output.rs`
  - `crates/gemini-cli/src/json.rs`
  - `crates/gemini-cli/src/jwt.rs`
  - `crates/gemini-cli/src/paths.rs`
  - `crates/gemini-cli/src/prompts.rs`
- **Description**: Port codex-style shared utility modules first so later feature modules can reuse
  parity-safe helpers for path resolution, JSON/JWT handling, prompt templates, and diagnostic
  formatting.
- **Dependencies**:
  - Task 3.1
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Shared utility behavior is deterministic and aligned with codex parity goals.
  - Utility modules consume `gemini-core` primitives where runtime logic is required.
  - Utility tests cover path, json, jwt, and prompt edge cases.
- **Validation**:
  - `cargo test -p nils-gemini-cli --test fs --test json --test jwt --test paths --test prompts`

### Task 3.3: Port auth command family with deterministic JSON and text behavior
- **Location**:
  - `crates/gemini-cli/src/auth/mod.rs`
  - `crates/gemini-cli/src/auth/login.rs`
  - `crates/gemini-cli/src/auth/use_secret.rs`
  - `crates/gemini-cli/src/auth/save.rs`
  - `crates/gemini-cli/src/auth/remove.rs`
  - `crates/gemini-cli/src/auth/refresh.rs`
  - `crates/gemini-cli/src/auth/auto_refresh.rs`
  - `crates/gemini-cli/src/auth/current.rs`
  - `crates/gemini-cli/src/auth/sync.rs`
  - `crates/gemini-cli/src/auth/output.rs`
- **Description**: Port full auth command family and JSON envelopes from codex parity baseline so
  text mode and machine mode remain stable for service consumers.
- **Dependencies**:
  - Task 3.1
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Every auth subcommand from parity matrix is implemented.
  - JSON and text outputs stay deterministic and avoid secret leakage.
  - Usage errors and confirmation-required flows return stable exit/code behavior.
- **Validation**:
  - `cargo test -p nils-gemini-cli --test auth_login --test auth_use --test auth_save --test auth_remove --test auth_refresh`
  - `cargo test -p nils-gemini-cli --test auth_auto_refresh --test auth_current_sync --test auth_json_contract --test auth_json_contract_more`

### Task 3.4: Port agent, diag/rate-limits, config, and starship command modules
- **Location**:
  - `crates/gemini-cli/src/agent/mod.rs`
  - `crates/gemini-cli/src/agent/exec.rs`
  - `crates/gemini-cli/src/agent/commit.rs`
  - `crates/gemini-cli/src/rate_limits/mod.rs`
  - `crates/gemini-cli/src/rate_limits/client.rs`
  - `crates/gemini-cli/src/rate_limits/render.rs`
  - `crates/gemini-cli/src/starship/mod.rs`
  - `crates/gemini-cli/src/starship/render.rs`
  - `crates/gemini-cli/src/config.rs`
- **Description**: Port remaining high-risk execution and diagnostics surfaces in focused modules so
  behavior can be validated per command family and regressions are isolated.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Agent, diag/rate-limits, config, and starship surfaces are implemented and deterministic.
  - Non-zero and network/error paths map to stable envelopes and exit codes.
  - Behavior stays compatible with parity matrix commitments.
- **Validation**:
  - `cargo test -p nils-gemini-cli --test agent_exec --test agent_prompt --test agent_commit --test agent_templates`
  - `cargo test -p nils-gemini-cli --test rate_limits_single --test rate_limits_all --test rate_limits_async --test rate_limits_render --test diag_json_contract`
  - `cargo test -p nils-gemini-cli --test starship_refresh --test starship_cached --test config`

### Task 3.5: Publish gemini-cli JSON contract and consumer runbook
- **Location**:
  - `crates/gemini-cli/docs/specs/gemini-cli-diag-auth-json-contract-v1.md`
  - `crates/gemini-cli/docs/runbooks/json-consumers.md`
  - `crates/gemini-cli/docs/README.md`
- **Description**: Define machine-consumable JSON contract for Gemini diag/auth surfaces with
  schema naming, stable envelope fields, compatibility rules, and no-secret-leak guarantees.
- **Dependencies**:
  - Task 1.1
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Schema versions and command paths are fully documented.
  - Error envelope stability rules and compatibility policy are explicit.
  - Consumer runbook contains concrete invocation and validation guidance.
- **Validation**:
  - `rg -n "gemini-cli\\.diag\\.rate-limits\\.v1|gemini-cli\\.auth\\.v1" crates/gemini-cli/docs/specs/gemini-cli-diag-auth-json-contract-v1.md`
  - `rg -n "schema_version|command|ok|error\\.code" crates/gemini-cli/docs/runbooks/json-consumers.md`

### Task 3.6: Add parity-oracle tests and completion assets in both shells
- **Location**:
  - `crates/gemini-cli/tests/main_entrypoint.rs`
  - `crates/gemini-cli/tests/dispatch.rs`
  - `crates/gemini-cli/tests/auth_json_contract.rs`
  - `crates/gemini-cli/tests/diag_json_contract.rs`
  - `crates/gemini-cli/tests/parity_oracle.rs`
  - `crates/gemini-cli/tests/completion_contract.rs`
  - `crates/gemini-cli/tests/completion_smoke.rs`
  - `completions/zsh/_gemini-cli`
  - `completions/bash/gemini-cli`
- **Description**: Add parity-oracle integration tests that compare Gemini command behavior to
  codex parity expectations (or explicit unsupported mappings), and ship thin completion adapters.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
  - Task 3.4
  - Task 3.5
- **Complexity**: 7
- **Acceptance criteria**:
  - Integration tests cover dispatch, JSON contract, and failure envelopes.
  - Parity-oracle tests assert section ordering, exit semantics, and envelope shape for mapped
    command families.
  - Completions are present in both shell directories and load generated output.
  - Completion tests pass without introducing alternate completion stacks.
- **Validation**:
  - `cargo test -p nils-gemini-cli --test main_entrypoint --test dispatch --test auth_json_contract --test diag_json_contract --test parity_oracle`
  - `cargo test -p nils-gemini-cli --test completion_contract --test completion_smoke`
  - `cargo run -p nils-gemini-cli -- completion zsh | rg -- "--help|--version|--format"`
  - `zsh -n completions/zsh/_gemini-cli`
  - `bash -n completions/bash/gemini-cli`
  - `zsh -f tests/zsh/completion.test.zsh`

## Sprint 4: Provider Stabilization and Agentctl Integration
**Goal**: replace Gemini stub adapter with stable runtime-backed implementation and integrate into
provider-neutral control plane.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-agent-provider-gemini`
  - `cargo test -p nils-agentctl --test provider_registry --test provider_commands`
  - `cargo run -p nils-agentctl -- provider list --format json`
- Verify:
  - Gemini appears as `stable`, with deterministic capability/health/execute/auth behavior.

### Task 4.1: Implement stable agent-provider-gemini adapter using gemini-core
- **Location**:
  - `crates/agent-provider-gemini/src/adapter.rs`
  - `crates/agent-provider-gemini/src/lib.rs`
  - `crates/agent-provider-gemini/Cargo.toml`
- **Description**: Replace stub logic with stable adapter behavior aligned to `provider-adapter.v1`,
  including metadata, capabilities, healthcheck, execute, limits, and auth-state mappings.
- **Dependencies**:
  - Task 2.1
  - Task 1.5
- **Complexity**: 8
- **Acceptance criteria**:
  - Adapter reports `ProviderMaturity::Stable`.
  - Execute path maps runtime failures to stable provider category/code taxonomy.
  - Adapter depends on `gemini-core`, not `gemini-cli`.
- **Validation**:
  - `cargo test -p nils-agent-provider-gemini --test adapter_contract`
  - `rg -n "gemini-core" crates/agent-provider-gemini/Cargo.toml`

### Task 4.2: Expand provider-gemini contract and boundary test coverage
- **Location**:
  - `crates/agent-provider-gemini/tests/adapter_contract.rs`
  - `crates/agent-provider-gemini/tests/dependency_boundary.rs`
  - `crates/agent-provider-gemini/tests/fixtures/README.md`
- **Description**: Replace stub-only tests with full contract coverage, add deterministic fixtures,
  and enforce dependency boundary constraints.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Adapter tests cover capability flags, health states, execute success/failure, limits, and
    auth-state transitions.
  - Fixture policy documents deterministic mock inputs and redaction rules.
  - Boundary test fails if forbidden CLI dependency/import is introduced.
- **Validation**:
  - `cargo test -p nils-agent-provider-gemini --test adapter_contract --test dependency_boundary`
  - `bash scripts/ci/gemini-core-boundary-check.sh`

### Task 4.3: Update agentctl registry and provider command expectations for stable Gemini
- **Location**:
  - `crates/agentctl/src/provider/registry.rs`
  - `crates/agentctl/tests/provider_registry.rs`
  - `crates/agentctl/tests/provider_commands.rs`
  - `crates/agentctl/README.md`
  - `crates/agent-runtime-core/README.md`
- **Description**: Update provider-neutral surfaces so Gemini is treated as stable in list and
  health outputs, and ensure registry/selection tests reflect the new maturity contract.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Builtin provider list shows `gemini` with `stable` maturity.
  - Provider command tests validate expected Gemini status/summary and selection behavior.
  - Runtime core and agentctl docs match actual maturity state.
- **Validation**:
  - `cargo test -p nils-agentctl --test provider_registry --test provider_commands --test diag_capabilities`
  - `cargo run -p nils-agentctl -- provider list --format json | rg "\"gemini\"|\"stable\""`

### Task 4.4: Add Gemini provider contract docs and verification oracle runbook
- **Location**:
  - `crates/agent-provider-gemini/README.md`
  - `crates/agent-provider-gemini/docs/README.md`
  - `crates/agent-provider-gemini/docs/specs/gemini-provider-contract-v1.md`
  - `crates/agent-provider-gemini/docs/runbooks/verification-oracles.md`
- **Description**: Document stable provider behavior, error taxonomy, fixture/oracle hierarchy, and
  release blocking criteria for Gemini provider changes.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Provider contract doc covers all `provider-adapter.v1` operations and compatibility rules.
  - Verification runbook defines mock and optional live drift checks.
  - Docs index references all new spec/runbook files.
- **Validation**:
  - `rg -n "Metadata|Operations|Error taxonomy|Compatibility" crates/agent-provider-gemini/docs/specs/gemini-provider-contract-v1.md`
  - `rg -n "mock|live|oracle|release blocker" crates/agent-provider-gemini/docs/runbooks/verification-oracles.md`

## Sprint 5: Hardening, Gating, and Release Readiness
**Goal**: finish cross-workspace docs/policy alignment, pass required checks, and produce a
reversible release gate.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - Required checks and coverage gate pass with Gemini lane enabled.

### Task 5.1: Align workspace docs and policy references with Gemini stable lane
- **Location**:
  - `README.md`
  - `BINARY_DEPENDENCIES.md`
  - `docs/reports/completion-coverage-matrix.md`
  - `docs/runbooks/provider-onboarding.md`
  - `docs/runbooks/codex-gemini-dual-cli-rollout.md`
- **Description**: Update workspace-level documentation so operator guidance, dependency policy,
  completion obligations, and provider onboarding requirements match the implemented Gemini state.
- **Dependencies**:
  - Task 1.4
  - Task 3.6
  - Task 4.4
- **Complexity**: 6
- **Acceptance criteria**:
  - No doc claims Gemini is still a stub.
  - Completion matrix stays consistent with workspace binaries and asset presence.
  - Onboarding runbook includes stable provider promotion requirements satisfied by Gemini.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `bash scripts/ci/completion-asset-audit.sh --strict`
  - `rg -n "gemini|stable|provider" README.md BINARY_DEPENDENCIES.md docs/runbooks/provider-onboarding.md`

### Task 5.2: Run full required checks and fix regressions until green
- **Location**:
  - `DEVELOPMENT.md`
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `target/coverage/lcov.info`
- **Description**: Execute mandatory repository checks and coverage gate for non-doc changes, then
  fix regressions in iterative passes until all gates are green.
- **Dependencies**:
  - Task 5.1
  - Task 2.2
  - Task 2.3
  - Task 3.6
  - Task 4.2
  - Task 4.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Required check entrypoint exits successfully.
  - Coverage run meets or exceeds `85.00%` line threshold.
  - Any Gemini-introduced failures are resolved without weakening existing contracts.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

### Task 5.3: Produce release cutover checklist and rollback drill artifacts
- **Location**:
  - `docs/reports/gemini-parity-cutover-checklist.md`
  - `docs/runbooks/codex-gemini-dual-cli-rollout.md`
  - `release/crates-io-publish-order.txt`
- **Description**: Capture operational cutover checklist, dependency-safe publish order validation,
  and explicit rollback drill steps so release can be executed and reversed safely.
- **Dependencies**:
  - Task 5.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Cutover checklist includes preflight, release, and post-release verification sections.
  - Dry-run publish command for Gemini crates is documented and reproducible.
  - Rollback drill includes actionable commands and expected post-rollback provider states.
- **Validation**:
  - `scripts/publish-crates.sh --dry-run --crates "nils-gemini-core nils-gemini-cli nils-agent-provider-gemini"`
  - `cargo run -p nils-agentctl -- provider healthcheck --provider gemini --format json`
  - `rg -n "^## Preflight|^## Release|^## Post-release verification|^## Rollback drill" docs/reports/gemini-parity-cutover-checklist.md`

## Testing Strategy
- Unit:
  - `gemini-core` module unit tests for config/auth/jwt/path/exec/error helpers.
  - `gemini-cli` module tests for prompt shaping, output envelopes, and formatter behavior.
  - `agent-provider-gemini` unit coverage for category/code mapping branches.
- Integration:
  - CLI integration tests for help/dispatch/exit code behavior and JSON contract invariants.
  - Provider integration tests for `capabilities`, `healthcheck`, `execute`, `limits`, and
    `auth-state`.
  - `agentctl` provider list/healthcheck tests with Gemini stable expectations.
- E2E/manual:
  - `agentctl provider list --format json` and `agentctl provider healthcheck --provider gemini`.
  - `gemini-cli` smoke commands in text and JSON mode.
  - Completion smoke (`zsh` and `bash`) and workspace completion matrix checks.

## Risks & gotchas
- Gemini runtime feature mismatch risk:
  - If Gemini runtime cannot provide Codex-equivalent rate-limit/auth semantics, parity can drift.
    Mitigation: lock unsupported behavior and stable codes in parity matrix and contract docs.
- Hidden dependency coupling risk:
  - Provider/CLI code can accidentally cross-import and violate architecture.
    Mitigation: boundary test + CI script in Sprint 2.
- Completion drift risk:
  - New binary without synchronized matrix/assets breaks completion audits.
    Mitigation: update matrix and both shell assets in same change set.
- Coverage regression risk:
  - Large code port can lower workspace coverage.
    Mitigation: add ported tests before merging and enforce `cargo llvm-cov` gate.

## Rollback plan
- Release rollback trigger:
  - Any blocker in provider contract tests, workspace required checks, or post-release healthcheck.
- Rollback actions:
  - Revert `agent-provider-gemini` maturity and behavior to deterministic stub contract.
  - Temporarily remove `nils-gemini-core` and `nils-gemini-cli` from release publish set while
    keeping code under feature branch for follow-up.
  - Restore docs state to the last known release tag for operator guidance consistency.
- Operational verification after rollback:
  - `cargo test -p nils-agent-provider-gemini --test adapter_contract`
  - `cargo run -p nils-agentctl -- provider list --format json | rg "\"gemini\"|\"stub\""`
  - `bash scripts/ci/docs-placement-audit.sh --strict`
