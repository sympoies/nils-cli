# Plan: macos-agent subcommand consistency and usability refactor

## Overview
This plan refactors `macos-agent` subcommands to make the CLI more user friendly, easier to discover, and more predictable in day-to-day agent workflows. The primary outcome is a clearer usage model (AX-first with explicit fallback boundaries) and consistent command contracts across output, retries, and metadata. The secondary outcome is maintainability: reducing duplicated CLI/backend logic, expanding coverage for compatibility-sensitive changes, and hardening stability through deterministic validation paths. The plan evaluates and implements all seven identified refactor directions, with backward-compatible rollout steps.

## Scope
- In scope:
  - Unify repeated output/format handling (`json` serialization branches, TSV rejection handling, text envelope patterns).
  - Standardize mutating command contract (`policy`, `meta`, retry/attempt telemetry) across AX and non-AX mutating commands.
  - Consolidate AX selector/target argument definitions into reusable `clap` structs.
  - Normalize naming and mental model for overlapping flags (`window_name` vs `window_title_contains`, `enter` vs `submit`) using compatibility aliases.
  - Clarify backend capability boundaries (`auto` vs Hammerspoon-only features) in runtime errors, preflight, and docs.
  - Deduplicate backend script helper logic in both `hammerspoon.rs` and `applescript.rs`.
  - Publish a usage decision matrix that makes command choice clear (`ax` vs `input`, fallback rules, troubleshooting path).
  - Add targeted tests to preserve backward compatibility and improve stability.
- Out of scope:
  - New automation domains outside current command families.
  - Non-macOS support.
  - Replacing AppleScript/Hammerspoon stacks with a new runtime.
  - Changing existing success/error exit code semantics.

## Assumptions (if any)
1. Backward compatibility for existing scripts is required; breaking flag/output changes must be migrated with aliases and explicit deprecation messaging.
2. The current AX-first direction remains the product strategy, while `input.*` stays as low-level fallback primitives.
3. `plan-tooling`, Rust toolchain, and required local test dependencies are available in contributor environments.
4. Mutating action telemetry should remain machine-parseable and stable for scenario/test harness consumers.

## Sprint 1: Contract and output consistency foundation
**Goal**: Remove repeated output/format boilerplate and make mutating command responses consistently parseable.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test cli_smoke`
- Verify:
  - Command outputs follow shared emit path with consistent error handling.
  - Mutating commands expose a uniform policy/meta contract.

**Parallelization notes**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.3` depends on `Task 1.1` and `Task 1.2`.

### Task 1.1: Extract shared command output helpers
- **Location**:
  - `crates/macos-agent/src/commands/mod.rs`
  - `crates/macos-agent/src/commands/ax_click.rs`
  - `crates/macos-agent/src/commands/ax_type.rs`
  - `crates/macos-agent/src/commands/ax_attr.rs`
  - `crates/macos-agent/src/commands/ax_action.rs`
  - `crates/macos-agent/src/commands/ax_session.rs`
  - `crates/macos-agent/src/commands/ax_watch.rs`
  - `crates/macos-agent/src/commands/input_click.rs`
  - `crates/macos-agent/src/commands/input_type.rs`
  - `crates/macos-agent/src/commands/input_hotkey.rs`
  - `crates/macos-agent/src/commands/input_source.rs`
  - `crates/macos-agent/src/commands/window_activate.rs`
  - `crates/macos-agent/src/commands/observe.rs`
  - `crates/macos-agent/src/commands/wait.rs`
  - `crates/macos-agent/src/commands/scenario.rs`
  - `crates/macos-agent/src/commands/profile.rs`
  - `crates/macos-agent/src/commands/list.rs`
  - `crates/macos-agent/src/error.rs`
- **Description**: Add shared helpers for JSON success emission, serialization error mapping, and unsupported TSV rejection. Replace per-command duplicated branches with helper calls while preserving current wire format.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Per-command repeated JSON serialization and TSV rejection branches are removed from handlers and replaced with shared helper usage.
  - CLI behavior and payload schema remain unchanged for existing commands.
  - Serialization and format errors remain operation-aware and actionable.
- **Validation**:
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test cli_smoke`

### Task 1.2: Unify mutating action envelope across command families
- **Location**:
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/commands/ax_attr.rs`
  - `crates/macos-agent/src/commands/ax_action.rs`
  - `crates/macos-agent/src/commands/ax_session.rs`
  - `crates/macos-agent/src/commands/ax_watch.rs`
- **Description**: Extend mutating AX commands that currently lack full action metadata to include consistent `policy` and `meta` blocks (including attempt semantics where relevant), matching existing `window.activate`, `input.*`, `ax.click`, `ax.type` behavior.
- **Dependencies**:
  - none
- **Complexity**: 8
- **Acceptance criteria**:
  - All mutating commands expose a documented, consistent policy/meta schema in JSON mode.
  - Dry-run responses preserve contract shape and explicit non-mutating semantics.
  - Existing consumers can parse mutating responses via one stable contract.
- **Validation**:
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test scenario_chain`

### Task 1.3: Consolidate command identity mapping used by dispatch and trace
- **Location**:
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/main.rs`
  - `crates/macos-agent/tests/cli_smoke.rs`
- **Description**: Introduce a shared command identity resolver used by runtime dispatch metadata and trace labeling to prevent drift between command routing and command label strings.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Command labels for tracing and JSON envelopes derive from a single source of truth.
  - New subcommands require changes in one mapping location only.
  - Existing trace/tests continue to pass without command label regressions.
- **Validation**:
  - `cargo test -p macos-agent --test cli_smoke`
  - `cargo test -p macos-agent --test contracts`

## Sprint 2: CLI UX normalization and discoverability
**Goal**: Make command usage rules easier to learn by reducing argument drift and harmonizing overlapping flag semantics.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --lib cli::tests`
  - `zsh -f tests/zsh/completion.test.zsh`
- Verify:
  - AX selector/target arguments are defined once and reused consistently.
  - Overlapping flags expose a clear canonical form plus compatibility aliases.

**Parallelization notes**:
- `Task 2.1` and `Task 2.2` can run in parallel.
- `Task 2.3` depends on `Task 2.1` and `Task 2.2`.

### Task 2.1: Introduce reusable AX `clap` argument fragments
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/commands/ax_common.rs`
  - `crates/macos-agent/src/commands/ax_click.rs`
  - `crates/macos-agent/src/commands/ax_type.rs`
  - `crates/macos-agent/src/commands/ax_attr.rs`
  - `crates/macos-agent/src/commands/ax_action.rs`
  - `crates/macos-agent/src/commands/ax_session.rs`
  - `crates/macos-agent/src/commands/ax_watch.rs`
  - `crates/macos-agent/src/commands/ax_list.rs`
- **Description**: Create reusable `AxTargetArgs` and `AxSelectorArgs` (or equivalent flattened structs), enforce shared validation constraints centrally, and apply them across `ax click/type/attr/action`.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - AX command parsing logic no longer duplicates selector/target field declarations.
  - Selector and target validation errors are consistent across AX subcommands.
  - Existing valid CLI invocations remain accepted.
- **Validation**:
  - `cargo test -p macos-agent --lib cli::tests`
  - `cargo test -p macos-agent --test cli_smoke`

### Task 2.2: Normalize user-facing flag vocabulary with backward-compatible aliases
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/commands/input_type.rs`
  - `crates/macos-agent/src/commands/wait.rs`
  - `crates/macos-agent/src/commands/observe.rs`
  - `completions/zsh/_macos-agent`
  - `completions/bash/macos-agent`
- **Description**: Define canonical naming for overlapping concepts (`window_title_contains`, submit/enter semantics) and keep old names as aliases during migration. Update help/completions so users see one preferred mental model.
- **Dependencies**:
  - none
- **Complexity**: 8
- **Acceptance criteria**:
  - Canonical flags are clearly documented in `--help` and completions.
  - Legacy flag forms continue to work and emit compatibility-safe messaging when appropriate.
  - Command behavior remains unchanged across alias and canonical paths.
- **Validation**:
  - `cargo test -p macos-agent --lib cli::tests`
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "window_title_contains|window-name|window_title" completions/bash/macos-agent`

### Task 2.3: Add explicit usage decision path to command help and docs
- **Location**:
  - `crates/macos-agent/README.md`
  - `crates/macos-agent/src/cli.rs`
- **Description**: Add concise decision guidance in CLI descriptions and README: when to start with `ax`, when to use fallback flags, and when to intentionally use `input.*` primitives.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Users can identify preferred command path without reading source code.
  - README and help text present consistent vocabulary and ordering.
  - Guidance covers stability-oriented flows (`activate` + `wait` + `ax` + fallback).
- **Validation**:
  - `rg -n "ax-first|fallback|input\.|decision" crates/macos-agent/README.md`
  - `cargo run -p macos-agent -- --help | rg -i "ax|fallback|input"`
  - `cargo run -p macos-agent -- --help`

## Sprint 3: Backend consistency and capability transparency
**Goal**: Reduce backend script duplication and make capability boundaries explicit to operators and tooling.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --lib backend::tests`
  - `cargo test -p macos-agent --test ax_extended`
- Verify:
  - Backend helper logic is centralized and easier to maintain.
  - Unsupported capability paths fail with explicit, actionable guidance.

**Parallelization notes**:
- `Task 3.1` can run in parallel with `Task 3.2`.
- `Task 3.3` depends on `Task 3.1` and `Task 3.2`.

### Task 3.1: Deduplicate Hammerspoon script helper blocks
- **Location**:
  - `crates/macos-agent/src/backend/hammerspoon.rs`
  - `crates/macos-agent/src/backend/mod.rs`
- **Description**: Factor repeated Lua helper fragments (`ensureState`, target/session resolution, selector walk utilities) into shared script prelude builders and operation-specific sections.
- **Dependencies**:
  - none
- **Complexity**: 9
- **Acceptance criteria**:
  - Shared helper logic is defined once and reused by AX operations.
  - Script behavior/output parity for all AX operations is preserved.
  - Existing backend tests remain green.
- **Validation**:
  - `cargo test -p macos-agent --lib backend::tests`
  - `cargo test -p macos-agent --test ax_extended`

### Task 3.2: Remove AppleScript/JXA helper duplication and tighten parser contracts
- **Location**:
  - `crates/macos-agent/src/backend/applescript.rs`
  - `crates/macos-agent/src/backend/mod.rs`
- **Description**: Eliminate duplicated JXA helper definitions (including repeated `resolveByNodeId`) and formalize parse/contract checks for backend JSON decoding.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - Duplicated JXA helper definitions are removed.
  - Parse failure messages remain operation-specific and actionable.
  - AX click/type/list behavior in test mode remains deterministic.
- **Validation**:
  - `cargo test -p macos-agent --lib backend::tests`
  - `cargo test -p macos-agent --test ax_extended`

### Task 3.3: Expose backend capability matrix in runtime diagnostics and preflight
- **Location**:
  - `crates/macos-agent/src/backend/mod.rs`
  - `crates/macos-agent/src/preflight.rs`
  - `crates/macos-agent/src/error.rs`
  - `crates/macos-agent/README.md`
- **Description**: Surface which AX features are available per backend preference (`auto`, `hammerspoon`, `applescript`) and provide explicit remediation for Hammerspoon-only commands (`attr/action/session/watch`).
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Unsupported backend command paths produce clear capability-specific hints.
  - Preflight (or documented diagnostics) can reveal backend readiness and likely fallback behavior.
  - README troubleshooting aligns with runtime diagnostics.
- **Validation**:
  - `cargo test -p macos-agent --test preflight`
  - `cargo test -p macos-agent --test contracts`

## Sprint 4: Stability hardening, test expansion, and rollout safety
**Goal**: Lock in usability and maintainability gains with focused coverage and operational rollout controls.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo test -p macos-agent --test scenario_chain`
- Verify:
  - Refactor remains backward compatible and stable.
  - Documentation, completions, and test suites reflect the new unified model.

**Parallelization notes**:
- `Task 4.1` and `Task 4.2` can run in parallel.
- `Task 4.3` depends on `Task 4.1` and `Task 4.2`.

### Task 4.1: Expand compatibility-focused test coverage
- **Location**:
  - `crates/macos-agent/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/tests/scenario_chain.rs`
  - `tests/zsh/completion.test.zsh`
- **Description**: Add tests for canonical-vs-alias flag parity, unified mutating envelope assertions, and scenario telemetry consistency for AX fallback paths.
- **Dependencies**:
  - Task 1.2
  - Task 2.2
  - Task 3.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Alias and canonical flags are behaviorally equivalent.
  - Contract tests enforce unified mutating output schema.
  - Scenario telemetry continues to encode fallback path details reliably.
- **Validation**:
  - `cargo test -p macos-agent --test cli_smoke`
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test scenario_chain`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.2: Publish user-facing command matrix and migration notes
- **Location**:
  - `crates/macos-agent/README.md`
  - `docs/plans/macos-agent-realtime-feedback-usability-hardening-plan.md`
- **Description**: Document a concrete command selection matrix (`ax`/`input`/`wait`/`window`) and migration notes for renamed canonical flags with alias compatibility windows.
- **Dependencies**:
  - Task 2.3
  - Task 3.3
- **Complexity**: 5
- **Acceptance criteria**:
  - README contains an actionable matrix and copy-paste workflow examples.
  - Migration notes explain compatibility guarantees and deprecation timeline.
  - Troubleshooting references updated backend capability guidance.
- **Validation**:
  - `rg -n "decision matrix|migration|alias|fallback|backend" crates/macos-agent/README.md`

### Task 4.3: Run full quality gate and prepare rollback-safe release handoff
- **Location**:
  - `DEVELOPMENT.md`
  - `crates/macos-agent/README.md`
  - `crates/macos-agent/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/tests/scenario_chain.rs`
  - `crates/macos-agent/tests/ax_extended.rs`
  - `crates/macos-agent/tests/preflight.rs`
- **Description**: Execute mandatory checks, collect residual risk notes, and prepare rollback procedure references so refactor rollout can be safely reverted without user disruption.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Required project checks pass or are explicitly documented with remediation.
  - Release handoff includes known-risk list and rollback trigger criteria.
  - No unresolved placeholder markers remain in command docs or plan artifacts.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit:
  - CLI parse/alias tests for selector/target normalization and backward compatibility.
  - Backend parser/contract tests for AX response decoding and capability errors.
- Integration:
  - Command contract tests for stdout/stderr discipline and JSON schema consistency.
  - Scenario chain tests for telemetry and fallback path integrity.
- E2E/manual:
  - Preflight + AX-first workflow smoke checks on local macOS hosts.
  - Manual verification of fallback behavior and troubleshooting clarity in README examples.

## Risks & gotchas
- Backward compatibility risk: scripts may rely on legacy flag names and current text output nuances.
- Refactor coupling risk: output/contract changes can affect scenario and external parsers simultaneously.
- Backend dedup risk: script refactors can silently alter AX resolution edge cases if not tightly tested.
- Usability drift risk: docs/help/completions can diverge unless updated in the same sprint as CLI changes.

## Rollback plan
- Keep refactor changes grouped by sprint and merge in small PRs so any regression can be reverted by sprint-level rollback.
- Preserve legacy flag aliases during rollout; if regressions appear, demote new canonical flags in docs/help and keep aliases as primary until fixed.
- If backend dedup introduces behavior regressions, revert backend script refactor commits first while retaining command-surface improvements.
- If mutating contract unification breaks consumers, temporarily restore previous JSON fields in compatibility mode and gate strict schema under opt-in until migration completes.
- Rollback trigger criteria:
  - Contract test regressions in `contracts` or `scenario_chain`.
  - Reproducible failures in AX command parity against pre-refactor baseline.
  - User-reported breakage in legacy flag paths.
