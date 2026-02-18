# Plan: Repo-Wide Completion Standard and Rollout

## Overview
This plan establishes a workspace-wide completion development standard, registers it in project `agent-docs` policy, and migrates completion behavior to a clap-first source of truth (`clap` + `clap_complete`) with thin shell adapters. The rollout starts with `codex-cli` as a pilot, then applies the same pattern to every remaining user-facing CLI that should ship completions. Existing command behavior and JSON/human output contracts remain unchanged; only completion architecture, tests, and contributor rules are updated.

## Scope
- In scope: Define a new workspace completion standard runbook and register it in `AGENT_DOCS.toml` (`project-dev`, required).
- In scope: Implement `codex-cli` clap-first completion export flow and migrate its zsh/bash completion adapters.
- In scope: Complete the same completion architecture for all remaining user-facing CLIs that require shipped completions.
- In scope: Add missing `image-processing` completion assets and tests if classified as completion-required.
- In scope: Expand completion validation, rollout runbooks, and no-legacy enforcement controls.
- Out of scope: Fish/PowerShell completion support.
- Out of scope: Behavioral changes to command execution semantics, output schemas, or exit-code contracts.
- Out of scope: Migrating `cli-template` into a user-facing release contract.

## Assumptions (if any)
1. Completion-required binaries are all workspace binaries except explicitly internal/example binaries (initially `cli-template`).
2. Each completion-required CLI can expose a user-facing completion export command (for example `completion <shell>`).
3. Completion adapters remain thin and completion behavior is clap-generated (no legacy completion-mode switch).
4. Completion files remain distributed under `completions/zsh/` and `completions/bash/`, with aliases synchronized in both shells.

## Sprint 1: Governance and Coverage Baseline
**Goal**: Create an enforceable repo-wide completion standard and bind it into project policy before implementation rollout.
**Demo/Validation**:
- Command(s): `python3 scripts/workspace-bins.py`
- Command(s): `agent-docs resolve --context project-dev --strict --format checklist`
- Verify: completion-required CLI matrix is explicit; new completion runbook is required by strict `project-dev` resolution.

### Task 1.1: Build completion coverage matrix for all workspace binaries
- **Location**:
  - `scripts/workspace-bins.py`
  - `tests/zsh/completion.test.zsh`
  - `completions/zsh/aliases.zsh`
  - `completions/bash/aliases.bash`
  - `docs/reports/completion-coverage-matrix.md` (new)
- **Description**: Inventory every workspace binary and classify completion obligation (`required` or `excluded`) with rationale, including explicit treatment for `image-processing` and `cli-template`.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Matrix enumerates every binary from `scripts/workspace-bins.py`.
  - Matrix records zsh/bash completion status and alias requirements per binary.
  - Exclusions are explicit and justified.
- **Validation**:
  - `python3 scripts/workspace-bins.py`
  - `rg -n "required|excluded|image-processing|cli-template" docs/reports/completion-coverage-matrix.md`

### Task 1.2: Author workspace completion development standard runbook
- **Location**:
  - `docs/runbooks/cli-completion-development-standard.md` (new)
  - `docs/runbooks/new-cli-crate-development-standard.md`
- **Description**: Define canonical completion architecture and rules: clap-first completion contract, shell adapter responsibilities, alias sync policy, no-legacy enforcement controls, context-aware candidate requirements, test gates, and release expectations.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Runbook includes required sections: architecture, contract boundaries, no-legacy policy, testing requirements, and rollout checklist.
  - New-CLI runbook references the completion standard as required guidance for future crates.
- **Validation**:
  - `rg -n "clap|clap_complete|adapter|no-legacy|aliases|context|testing" docs/runbooks/cli-completion-development-standard.md`
  - `rg -n "cli-completion-development-standard" docs/runbooks/new-cli-crate-development-standard.md`

### Task 1.3: Register completion standard in project agent-docs policy
- **Location**:
  - `AGENT_DOCS.toml`
- **Description**: Add the new runbook as a required `project-dev` document so strict policy resolution enforces completion governance before implementation work.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - `AGENT_DOCS.toml` includes a required `project-dev` entry for `docs/runbooks/cli-completion-development-standard.md`.
  - `agent-docs resolve --context project-dev --strict --format checklist` reports the document as `present`.
- **Validation**:
  - `agent-docs resolve --context project-dev --strict --format checklist`
  - `rg -n "cli-completion-development-standard.md" AGENT_DOCS.toml`

### Task 1.4: Align top-level contributor docs with completion governance
- **Location**:
  - `AGENTS.md`
  - `DEVELOPMENT.md`
  - `README.md`
- **Description**: Cross-link and align completion workflow references so contributor onboarding, required checks, and release packaging point to one canonical completion standard.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Top-level docs consistently reference the new completion runbook without conflicting instructions.
  - Required validation commands remain consistent with repository gates.
- **Validation**:
  - `rg -n "completion|cli-completion-development-standard|tests/zsh/completion.test.zsh" AGENTS.md DEVELOPMENT.md README.md`

## Sprint 2: Codex-CLI Pilot (Architecture Proving Ground)
**Goal**: Implement and validate the new completion architecture on `codex-cli` end-to-end before scaling to the rest of the workspace.
**Demo/Validation**:
- Command(s): `cargo test -p nils-codex-cli`
- Command(s): `zsh -f tests/zsh/completion.test.zsh`
- Verify: `codex-cli` completion is clap-first, context-aware (not global candidate dumps), aliases preserve behavior, and no legacy-mode gate remains.

### Task 2.1: Add `completion <shell>` export path for codex-cli
- **Location**:
  - `crates/codex-cli/src/cli.rs`
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/src/completion/mod.rs` (new)
- **Description**: Extend CLI parsing/dispatch with a user-facing completion export command powered by `clap_complete`, while keeping command behavior unchanged.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - `codex-cli completion zsh` and `codex-cli completion bash` emit deterministic completion scripts.
  - `codex-cli --help` exposes the completion export entrypoint.
- **Validation**:
  - `cargo run -p nils-codex-cli --bin codex-cli -- completion zsh >/dev/null`
  - `cargo run -p nils-codex-cli --bin codex-cli -- completion bash >/dev/null`
  - `bash -lc 'set -euo pipefail; cargo run -p nils-codex-cli --bin codex-cli -- --help | rg -q "completion"'`
  - `cargo test -p nils-codex-cli main_entrypoint`

### Task 2.2: Encode codex-cli completion metadata in clap + optional dynamic value providers
- **Location**:
  - `crates/codex-cli/src/cli.rs`
  - `crates/codex-cli/src/completion/mod.rs`
  - `crates/codex-cli/tests/completion_contract.rs` (new)
- **Description**: Ensure clap definitions express subcommands, long/short flags, and declared values (`ValueEnum`/`PossibleValue`/`ValueHint`) so completion stays context-aware; add optional dynamic providers via `clap_complete::env::CompleteEnv` only where runtime values are necessary.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Candidate sets cover `agent/auth/diag/config/starship` and nested command paths.
  - Option/value completions match clap contract (including `--format text|json`) and are cursor-position aware.
  - Any dynamic value provider remains deterministic and non-blocking.
- **Validation**:
  - `cargo test -p nils-codex-cli completion_contract`
  - `cargo run -p nils-codex-cli --bin codex-cli -- completion zsh | rg -q -- "--format"`

### Task 2.3: Convert codex-cli zsh/bash files to thin adapters (no legacy gate)
- **Location**:
  - `completions/zsh/_codex-cli`
  - `completions/bash/codex-cli`
  - `completions/zsh/aliases.zsh`
  - `completions/bash/aliases.bash`
- **Description**: Replace hardcoded completion matrices with clap-generated assets, preserve alias injection semantics (`cx*`, `cxdra`), and remove `CODEX_CLI_COMPLETION_MODE=legacy` rollback path.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Shell completions are generated from `codex-cli completion zsh|bash`, not manually curated flag lists.
  - Alias/wrapper invocation behavior remains compatible with current `cx*` UX.
  - No `CODEX_CLI_COMPLETION_MODE` gate or legacy completion function remains in codex adapters.
  - Shell files keep only adapter-specific logic and alias wiring.
- **Validation**:
  - `zsh -n completions/zsh/_codex-cli`
  - `bash -n completions/bash/codex-cli`
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "CODEX_CLI_COMPLETION_MODE|_nils_cli_codex_cli_complete_legacy" completions/zsh/_codex-cli completions/bash/codex-cli`

### Task 2.4: Add pilot-specific completion regression tests
- **Location**:
  - `tests/zsh/completion.test.zsh`
  - `crates/codex-cli/tests/main_entrypoint.rs`
  - `crates/codex-cli/tests/completion_smoke.rs` (new)
- **Description**: Add regression checks for export-command stability, adapter wiring, alias mapping, no-legacy enforcement, and context-aware candidate filtering.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests fail on alias drift, missing completion registration, or context-insensitive candidate regressions.
  - Existing completion assertions remain green without loosening constraints.
- **Validation**:
  - `cargo test -p nils-codex-cli`
  - `zsh -f tests/zsh/completion.test.zsh`

## Sprint 3: Rollout Framework for Workspace Migration
**Goal**: Build reusable migration scaffolding so remaining CLI conversions are consistent and low-risk.
**Demo/Validation**:
- Command(s): `zsh -f tests/zsh/completion.test.zsh`
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
- Verify: reusable adapter/testing framework exists and migration checklist is executable.

### Task 3.1: Introduce shared adapter helper patterns for bash/zsh
- **Location**:
  - `completions/zsh/_completion-adapter-common.zsh` (new)
  - `completions/bash/completion-adapter-common.bash` (new)
  - `docs/runbooks/cli-completion-development-standard.md`
- **Description**: Create shared helper functions for loading clap-generated completion assets, handling shell quoting/registration, enforcing no-legacy adapter behavior, and reducing per-file duplication across CLI completion adapters.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 7
- **Acceptance criteria**:
  - At least one migrated CLI adapter consumes shared helpers.
  - Helper contract and shell compatibility caveats are documented.
- **Validation**:
  - `zsh -n completions/zsh/_completion-adapter-common.zsh`
  - `bash -n completions/bash/completion-adapter-common.bash`

### Task 3.2: Add per-CLI migration checklist and completion contract template
- **Location**:
  - `docs/runbooks/cli-completion-development-standard.md`
  - `docs/specs/completion-contract-template.md` (new)
- **Description**: Define a repeatable per-CLI contract template (command graph, value providers, alias map, no-legacy invariants, tests) to keep all migrations consistent.
- **Dependencies**:
  - Task 1.2
  - Task 3.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Template includes required fields for implementation and test coverage.
  - Migration checklist is actionable without undocumented steps.
- **Validation**:
  - `rg -n "alias map|no-legacy|validation|acceptance" docs/specs/completion-contract-template.md docs/runbooks/cli-completion-development-standard.md`

### Task 3.3: Expand completion test harness to be coverage-driven
- **Location**:
  - `tests/zsh/completion.test.zsh`
  - `tests/completion/coverage_matrix.sh` (new)
  - `docs/reports/completion-coverage-matrix.md`
- **Description**: Convert ad-hoc completion checks into matrix-driven assertions keyed by completion-required CLIs so additions/removals are explicit and test-enforced.
- **Dependencies**:
  - Task 1.1
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Test harness fails when a completion-required CLI lacks zsh/bash assets or registration checks.
  - Matrix file drives test expectations rather than hardcoded assumptions.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `bash tests/completion/coverage_matrix.sh`

### Task 3.4: Standardize and test no-legacy completion enforcement contract
- **Location**:
  - `docs/reports/completion-coverage-matrix.md`
  - `docs/runbooks/cli-completion-development-standard.md`
  - `tests/completion/coverage_matrix.sh`
- **Description**: Define no-legacy completion enforcement metadata and require each completion-required CLI migration to declare how adapters stay clap-first without completion mode toggles.
- **Dependencies**:
  - Task 1.1
  - Task 3.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Every completion-required CLI has explicit no-legacy completion enforcement metadata in the matrix/template.
  - Matrix-driven checks fail if no-legacy metadata is missing.
- **Validation**:
  - `rg -n "no-legacy|COMPLETION_MODE|legacy completion mode" docs/reports/completion-coverage-matrix.md docs/specs/completion-contract-template.md`
  - `bash tests/completion/coverage_matrix.sh`

## Sprint 4: Full Migration for Remaining Completion-Required CLIs
**Goal**: Finish completion architecture migration for each remaining CLI as atomic, verifiable units after codex-cli pilot validation.
**Demo/Validation**:
- Command(s): `cargo test --workspace`
- Command(s): `zsh -f tests/zsh/completion.test.zsh`
- Verify: every completion-required CLI has clap-first completion contract + thin zsh/bash adapter coverage.

### Task 4.1: Migrate `git-scope` completion architecture
- **Location**:
  - `crates/git-scope/src/main.rs`
  - `completions/zsh/_git-scope`
  - `completions/bash/git-scope`
  - `completions/zsh/aliases.zsh`
  - `completions/bash/aliases.bash`
- **Description**: Add clap-based completion export path (with optional dynamic extension) and migrate shell adapters for `git-scope` while preserving `gs*` alias compatibility.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `git-scope` completion candidates are clap-first.
  - `gs*` alias completion behavior remains stable.
- **Validation**:
  - `cargo test -p nils-git-scope`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.2: Migrate `git-summary` completion architecture
- **Location**:
  - `crates/git-summary/src/cli.rs`
  - `completions/zsh/_git-summary`
  - `completions/bash/git-summary`
- **Description**: Implement clap-first completion contract and migrate zsh/bash adapters for `git-summary`.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - `git-summary` completion contract is clap-first and deterministic.
- **Validation**:
  - `cargo test -p nils-git-summary`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.3: Migrate `git-lock` completion architecture
- **Location**:
  - `crates/git-lock/src/main.rs`
  - `completions/zsh/_git-lock`
  - `completions/bash/git-lock`
- **Description**: Implement clap-first completion command and adapter migration for `git-lock` option/subcommand surfaces.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - `git-lock` completion behavior remains contract-accurate and clap-first.
- **Validation**:
  - `cargo test -p nils-git-lock`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.4: Migrate `git-cli` completion architecture
- **Location**:
  - `crates/git-cli/src/main.rs`
  - `completions/zsh/_git-cli`
  - `completions/bash/git-cli`
  - `completions/zsh/aliases.zsh`
  - `completions/bash/aliases.bash`
- **Description**: Implement clap-first completion contract and adapter migration for `git-cli` while preserving `gx*` alias injection mappings.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 8
- **Acceptance criteria**:
  - `git-cli` completion contract is clap-first.
  - `gx*` alias mapping remains equivalent to current behavior.
- **Validation**:
  - `cargo test -p nils-git-cli`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.5: Migrate `api-rest` completion architecture
- **Location**:
  - `crates/api-rest/src/cli.rs`
  - `completions/zsh/_api-rest`
  - `completions/bash/api-rest`
- **Description**: Migrate `api-rest` to clap-first completion contract with thin shell adapters and default-subcommand compatibility.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `api-rest` completion remains contract-accurate for nested options and value flags.
- **Validation**:
  - `cargo test -p nils-api-rest`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.6: Migrate `api-gql` completion architecture
- **Location**:
  - `crates/api-gql/src/cli.rs`
  - `completions/zsh/_api-gql`
  - `completions/bash/api-gql`
- **Description**: Migrate `api-gql` to clap-first completion contract with thin shell adapters and schema/report command coverage.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `api-gql` completion remains contract-accurate for nested options and value flags.
- **Validation**:
  - `cargo test -p nils-api-gql`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.7: Migrate `api-grpc` completion architecture
- **Location**:
  - `crates/api-grpc/src/cli.rs`
  - `completions/zsh/_api-grpc`
  - `completions/bash/api-grpc`
- **Description**: Migrate `api-grpc` completion to clap-generated baseline + shell adapters with call/history/report parity.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `api-grpc` completion is clap-first and parity-stable.
- **Validation**:
  - `cargo test -p nils-api-grpc`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.8: Migrate `api-websocket` completion architecture
- **Location**:
  - `crates/api-websocket/src/cli.rs`
  - `completions/zsh/_api-websocket`
  - `completions/bash/api-websocket`
- **Description**: Migrate `api-websocket` completion to clap-generated baseline + shell adapters for call/history/report surfaces.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `api-websocket` completion is clap-first and parity-stable.
- **Validation**:
  - `cargo test -p nils-api-websocket`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.9: Migrate `api-test` completion architecture
- **Location**:
  - `crates/api-test/src/main.rs`
  - `completions/zsh/_api-test`
  - `completions/bash/api-test`
- **Description**: Migrate `api-test` completion to clap-generated baseline + shell adapters, including default `run` and `summary` paths.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - `api-test` completion is clap-first and parity-stable.
- **Validation**:
  - `cargo test -p nils-api-test`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.10: Migrate `agent-docs` completion architecture
- **Location**:
  - `crates/agent-docs/src/cli.rs`
  - `completions/zsh/_agent-docs`
  - `completions/bash/agent-docs`
- **Description**: Migrate `agent-docs` completion to clap-generated baseline + shell adapters while preserving format/options behavior.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `agent-docs` completion remains contract-accurate and clap-first.
- **Validation**:
  - `cargo test -p nils-agent-docs`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.11: Migrate `agentctl` completion architecture
- **Location**:
  - `crates/agentctl/src/cli.rs`
  - `completions/zsh/_agentctl`
  - `completions/bash/agentctl`
- **Description**: Migrate `agentctl` completion to clap-generated baseline + thin adapters for provider/diag/debug/workflow command trees.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `agentctl` completion remains contract-accurate and clap-first.
- **Validation**:
  - `cargo test -p nils-agentctl`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.12: Migrate `memo-cli` completion architecture
- **Location**:
  - `crates/memo-cli/src/cli.rs`
  - `completions/zsh/_memo-cli`
  - `completions/bash/memo-cli`
- **Description**: Migrate `memo-cli` completion to clap-generated baseline + thin adapters, preserving search/report/fetch/apply option coverage.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `memo-cli` completion remains contract-accurate and clap-first.
- **Validation**:
  - `cargo test -p nils-memo-cli`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.13: Migrate `macos-agent` completion architecture
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `completions/zsh/_macos-agent`
  - `completions/bash/macos-agent`
- **Description**: Migrate `macos-agent` completion to clap-generated baseline + thin adapters while preserving canonical/deprecated flag alias guidance.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `macos-agent` completion preserves canonical+alias flag contract and is clap-first.
- **Validation**:
  - `cargo test -p nils-macos-agent`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.14: Migrate `plan-tooling` completion architecture
- **Location**:
  - `crates/plan-tooling/src/main.rs`
  - `completions/zsh/_plan-tooling`
  - `completions/bash/plan-tooling`
- **Description**: Migrate `plan-tooling` completion to clap-generated baseline + thin adapters for parse/validate/batches/scaffold flows.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - `plan-tooling` completion is clap-first and contract-accurate.
- **Validation**:
  - `cargo test -p nils-plan-tooling`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.15: Migrate `semantic-commit` completion architecture
- **Location**:
  - `crates/semantic-commit/src/main.rs`
  - `completions/zsh/_semantic-commit`
  - `completions/bash/semantic-commit`
- **Description**: Migrate `semantic-commit` completion to clap-generated baseline + thin adapters for staged-context/commit flows.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - `semantic-commit` completion is clap-first and parity-stable.
- **Validation**:
  - `cargo test -p nils-semantic-commit`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.16: Migrate `fzf-cli` completion architecture
- **Location**:
  - `crates/fzf-cli/src/main.rs`
  - `completions/zsh/_fzf-cli`
  - `completions/bash/fzf-cli`
  - `completions/zsh/aliases.zsh`
  - `completions/bash/aliases.bash`
- **Description**: Migrate `fzf-cli` completion to clap-generated baseline + thin adapters while preserving `fx*` alias/function behavior.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `fzf-cli` completion is clap-first.
  - `fx*` alias/function completion behavior remains stable.
- **Validation**:
  - `cargo test -p nils-fzf-cli`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.17: Migrate `screen-record` completion architecture
- **Location**:
  - `crates/screen-record/src/cli.rs`
  - `completions/zsh/_screen-record`
  - `completions/bash/screen-record`
- **Description**: Migrate `screen-record` completion to clap-generated baseline + thin adapters with platform-specific option parity.
- **Dependencies**:
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - `screen-record` completion is clap-first and parity-stable.
- **Validation**:
  - `cargo test -p nils-screen-record`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.18: Resolve `image-processing` completion status and implement accordingly
- **Location**:
  - `crates/image-processing/src/cli.rs`
  - `completions/zsh/_image-processing` (new)
  - `completions/bash/image-processing` (new)
  - `docs/reports/completion-coverage-matrix.md`
  - `tests/zsh/completion.test.zsh`
- **Description**: Use matrix policy to finalize whether `image-processing` is completion-required; if required, implement clap-generated baseline + thin adapters; if excluded, codify exclusion rationale and enforce it in matrix-driven tests.
- **Dependencies**:
  - Task 1.1
  - Task 3.3
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - `image-processing` status is explicit, test-enforced, and aligned with policy.
  - Required path: clap-first completion + zsh/bash assets.
  - Excluded path: documented rationale + enforced exclusion check in matrix tests.
- **Validation**:
  - `cargo test -p nils-image-processing`
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "image-processing" docs/reports/completion-coverage-matrix.md tests/completion/coverage_matrix.sh`

## Sprint 5: Hardening, CI Gates, and Release Readiness
**Goal**: Lock in migration with reproducible checks, contributor guidance, and no-legacy completion governance.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Command(s): `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
- Verify: required checks, completion coverage, and no-legacy controls are release-ready.

### Task 5.1: Update release/integration runbooks for new completion architecture
- **Location**:
  - `DEVELOPMENT.md`
  - `docs/runbooks/INTEGRATION_TEST.md`
  - `README.md`
- **Description**: Document completion verification commands, no-legacy completion policy, and release packaging expectations for completion assets.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
  - Task 4.3
  - Task 4.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Contributor docs include explicit completion verification and no-legacy instructions.
  - Release documentation reflects final completion asset layout.
- **Validation**:
  - `rg -n "completion|clap_complete|no-legacy|legacy completion mode" DEVELOPMENT.md docs/runbooks/INTEGRATION_TEST.md README.md`

### Task 5.2: Execute required checks and workspace coverage gate
- **Location**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `scripts/ci/coverage-summary.sh`
- **Description**: Run full required lint/test/completion checks and coverage threshold, then capture failures/remediation if any.
- **Dependencies**:
  - Task 5.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Required check entrypoint completes successfully.
  - Coverage report meets `>= 85.00%` line threshold.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

### Task 5.3: Add release artifact completion audit
- **Location**:
  - `scripts/ci/completion-asset-audit.sh` (new)
  - `docs/reports/completion-coverage-matrix.md`
- **Description**: Add a CI/release audit script that asserts completion assets for all completion-required CLIs are present in both shell directories and aligned with matrix policy.
- **Dependencies**:
  - Task 1.1
  - Task 4.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Audit fails on missing zsh/bash pair, undeclared exclusions, or matrix drift.
  - Script can run in CI and local verification flows.
- **Validation**:
  - `bash scripts/ci/completion-asset-audit.sh --strict`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 5.4: Wire completion audit into required-check and CI workflows
- **Location**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `.github/workflows/ci.yml`
  - `scripts/ci/completion-asset-audit.sh`
- **Description**: Integrate completion asset audit into required local/CI gates so completion coverage drift is blocked by default.
- **Dependencies**:
  - Task 5.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Required check entrypoint runs completion audit in full mode.
  - CI workflow executes completion audit as part of the mandatory pipeline.
- **Validation**:
  - `rg -n "completion-asset-audit\\.sh" ./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh .github/workflows/ci.yml`
  - `bash scripts/ci/completion-asset-audit.sh --strict`

## Dependency & Parallelization Map
- Critical path: `1.1 -> 1.2 -> 1.3 -> 2.1 -> 2.2 -> 2.3 -> 2.4 -> 3.1 -> 3.2 -> 3.3 -> 3.4 -> 4.1..4.18 -> 5.1 -> 5.3 -> 5.4 -> 5.2`.
- Parallelizable after Task 3.4:
  - Task 4.1 through Task 4.18 can run in parallel per CLI (subject to merge/conflict coordination).
- Parallelizable in hardening:
  - Task 5.1 can run in parallel with late-stage Sprint 4 tasks once architecture is stable.
  - Task 5.3 can start once Task 1.1 and Task 4.18 are stable.

## Testing Strategy
- Unit:
  - Per-crate completion contract tests (clap command model, exported script smoke, candidate/value routing).
  - Alias injection and no-legacy enforcement tests where alias semantics are non-trivial.
- Integration:
  - `tests/zsh/completion.test.zsh` matrix-driven completion asset and registration checks.
  - Crate integration tests for completion export-command stability and context-aware candidate behavior.
- E2E/manual:
  - Spot-check tab completion for nested commands and value flags across representative CLI families.
  - No-legacy drill: verify adapters contain no completion-mode switches and generated path remains context-aware.

## Risks & gotchas
- Per-CLI migration scope is large; drift between matrix policy and implemented adapters can regress silently without strict audit.
- Shell quoting/word-splitting differences can break one shell while tests pass in the other if adapter helpers are incomplete.
- Completion quality may regress to global candidate dumps if clap metadata does not fully encode subcommand/flag/value constraints.
- Completion latency may regress if optional runtime value providers do expensive checks on every tab press.
- Alias-heavy CLIs (`git-cli`, `codex-cli`, `fzf-cli`) are prone to behavioral regressions if injection semantics diverge from current contracts.

## Regression response plan
- Do not reintroduce legacy completion mode toggles (`*_COMPLETION_MODE`) during rollout.
- If a migrated CLI regresses, patch clap metadata and/or thin adapter wiring directly, then ship a focused hotfix.
- If a family-wide regression appears, revert only the affected completion family files plus related crate completion module changes, then rerun required checks.
- Keep matrix/test metadata as the enforcement source-of-truth so no-legacy invariants remain auditable.
