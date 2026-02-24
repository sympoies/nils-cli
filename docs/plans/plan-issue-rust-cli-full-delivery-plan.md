# Plan: Rust Plan-Issue CLI Full Delivery

## Sprint 1: Contract and parity fixtures

### Task 1.1: Write Rust CLI contract v1 and command matrix
- **Location**:
  - crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md
- **Description**: Define v1 command matrix, global flags, and output contracts for the Rust CLI.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Contract spec documents command surface and option semantics.
- **Validation**:
  - test -f crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md

### Task 1.2: Capture baseline shell fixtures for parity assertions
- **Location**:
  - crates/plan-issue-cli/tests/fixtures/shell_parity/help.txt
- **Description**: Capture deterministic shell parity fixtures used by Rust compatibility assertions.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Fixture corpus includes help and guide baselines.
- **Validation**:
  - test -f crates/plan-issue-cli/tests/fixtures/shell_parity/help.txt

### Task 1.3: Define state machine and gate invariants
- **Location**:
  - crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md
  - crates/plan-issue-cli/docs/specs/plan-issue-gate-matrix-v1.md
- **Description**: Define gate ordering and status transitions for plan/sprint orchestration.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - State and gate specs are documented with deterministic rules.
- **Validation**:
  - test -f crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md

## Sprint 2: Rust CLI scaffold and baseline envelopes

### Task 2.1: Scaffold `nils-plan-issue-cli` crate with two binaries
- **Location**:
  - crates/plan-issue-cli/Cargo.toml
  - crates/plan-issue-cli/src/main.rs
  - crates/plan-issue-cli/src/bin/plan-issue-local.rs
- **Description**: Create workspace crate and binary entrypoints for live and local modes.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Crate builds with both binaries registered.
- **Validation**:
  - cargo check -p nils-plan-issue-cli

### Task 2.2: Implement clap command tree and typed argument model
- **Location**:
  - crates/plan-issue-cli/src/cli.rs
  - crates/plan-issue-cli/src/commands/
- **Description**: Implement typed command/subcommand and argument parsing for v1 command surface.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Help output and grouping args match command contract.
- **Validation**:
  - cargo test -p nils-plan-issue-cli cli_help

### Task 2.3: Add output envelope for text and JSON modes
- **Location**:
  - crates/plan-issue-cli/src/output/
  - crates/plan-issue-cli/tests/output_contract.rs
- **Description**: Add deterministic text and JSON envelope outputs for success and error responses.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Output envelope is stable and machine-consumable.
- **Validation**:
  - cargo test -p nils-plan-issue-cli output_json_contract

## Sprint 3: Rendering and local workflow core

### Task 3.1: Implement task-spec generation core using `plan-tooling`
- **Location**:
  - crates/plan-issue-cli/src/task_spec.rs
  - crates/plan-issue-cli/src/execute.rs
- **Description**: Generate deterministic sprint/plan task-spec TSV artifacts from plan markdown using shared plan-tooling parsing core.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Task-spec generation supports both grouping modes and deterministic note fields.
- **Validation**:
  - cargo test -p nils-plan-issue-cli task_spec_generation

### Task 3.2: Implement issue-body and sprint-comment rendering engine
- **Location**:
  - crates/plan-issue-cli/src/render.rs
  - crates/plan-issue-cli/src/execute.rs
- **Description**: Render plan issue body and sprint comment markdown from task-spec + plan context.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Start/ready/accepted comment modes render deterministic tables and headings.
- **Validation**:
  - cargo test -p nils-plan-issue-cli render_issue_body

### Task 3.3: Implement independent local dry-run workflow
- **Location**:
  - crates/plan-issue-cli/src/lib.rs
  - crates/plan-issue-cli/src/execute.rs
- **Description**: Implement local-first dry-run command execution flow without GitHub side effects.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - `plan-issue-local --dry-run` supports end-to-end artifact generation flow.
- **Validation**:
  - cargo test -p nils-plan-issue-cli local_flow

## Sprint 4: Live adapter and orchestration

### Task 4.1: Implement GitHub adapter abstraction and `gh` backend
- **Location**:
  - crates/plan-issue-cli/src/github/
- **Description**: Add adapter abstraction and initial `gh`-backed implementation.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Adapter surface supports issue read/write and PR status lookup.
- **Validation**:
  - cargo test -p nils-plan-issue-cli github_adapter

### Task 4.2: Implement live plan-level commands
- **Location**:
  - crates/plan-issue-cli/src/execute.rs
- **Description**: Implement live start/status/ready/close plan command flow.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Plan-level live commands enforce gate invariants and lifecycle updates.
- **Validation**:
  - cargo test -p nils-plan-issue-cli live_plan_commands

### Task 4.3: Implement live sprint-level commands and guide output
- **Location**:
  - crates/plan-issue-cli/src/execute.rs
- **Description**: Implement live start/ready/accept sprint commands and multi-sprint guide output.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Sprint-level live commands enforce sequencing and merge gates.
- **Validation**:
  - cargo test -p nils-plan-issue-cli live_sprint_commands

## Sprint 5: Guardrails and compatibility hardening

### Task 5.1: Add parity regression suite against shell fixtures
- **Location**:
  - crates/plan-issue-cli/tests/fixtures/shell_parity/
  - crates/plan-issue-cli/tests/
- **Description**: Add regression suite to compare Rust outputs against shell fixture snapshots.
- **Dependencies**:
  - Task 4.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Parity suite catches user-visible drift in key outputs.
- **Validation**:
  - cargo test -p nils-plan-issue-cli parity_shell

### Task 5.2: Implement command guardrails and preflight checks
- **Location**:
  - crates/plan-issue-cli/src/execute.rs
- **Description**: Add command-level guardrails for missing inputs and invalid operation modes.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Guardrails emit stable structured error codes/messages.
- **Validation**:
  - cargo test -p nils-plan-issue-cli command_guardrails

### Task 5.3: Add contract-level JSON compatibility tests
- **Location**:
  - crates/plan-issue-cli/tests/output_contract.rs
- **Description**: Add compatibility-focused JSON assertions for machine-consumed command envelopes.
- **Dependencies**:
  - Task 4.3
- **Complexity**: 5
- **Acceptance criteria**:
  - JSON contract tests cover success/failure envelopes and required fields.
- **Validation**:
  - cargo test -p nils-plan-issue-cli json_contract

## Sprint 6: Skill cutover and final acceptance

### Task 6.1: Wire skill entrypoint to Rust CLI and keep compatibility wrapper
- **Location**:
  - wrappers/plan-issue-delivery-loop.sh
- **Description**: Move skill entrypoint to Rust CLI while preserving compatibility wrapper behavior.
- **Dependencies**:
  - Task 5.1
  - Task 5.2
  - Task 5.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Wrapper delegates to Rust CLI without behavioral regressions.
- **Validation**:
  - test -f wrappers/plan-issue-delivery-loop.sh

### Task 6.2: Update completion, aliases, and user docs
- **Location**:
  - completions/zsh/
  - completions/bash/
  - README.md
- **Description**: Add completion assets and documentation for new Rust CLI entrypoints.
- **Dependencies**:
  - Task 5.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Completion scripts and docs match command surface and rollout policy.
- **Validation**:
  - zsh -n completions/zsh/_plan-issue

### Task 6.3: Final verification and sprint-6 acceptance artifact
- **Location**:
  - $AGENT_HOME/out/plan-issue-rust-cli/sprint-6/acceptance.md
- **Description**: Run final required checks and write acceptance artifact for the full delivery plan.
- **Dependencies**:
  - Task 6.1
  - Task 6.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Required checks pass and acceptance artifact is recorded.
- **Validation**:
  - ./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
