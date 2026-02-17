# Plan: macos-agent subcommand consistency and usability execution plan

## Overview
This plan operationalizes `docs/plans/macos-agent-subcommand-consistency-usability-refactor-plan.md` into an executable sprint schedule with lane ownership, effort estimates, dependency gates, and rollout controls. The primary goal is user-friendly, clear, and easy-to-use CLI behavior; the secondary goal is maintainability through lower duplication, higher test coverage, and stronger stability guarantees. The plan is designed for incremental delivery with compatibility-preserving checkpoints after every sprint. Default capacity is two engineers plus shared documentation/testing support.

## Scope
- In scope:
  - Convert strategy tasks into sequenced execution tasks with estimated effort and lane ownership.
  - Define critical path, parallelizable work, and integration checkpoints.
  - Add explicit sprint Go/No-Go criteria and validation commands.
  - Provide dual capacity guidance (2-engineer default, 1-engineer fallback).
- Out of scope:
  - Implementing the refactor itself in this plan document.
  - Changing release branching policy beyond this refactor.

## Assumptions (if any)
1. Default staffing model: 2 engineers (Lane A + Lane B) with shared support for docs/completions/tests.
2. If only 1 engineer is available, the same task order applies but parallel tasks become sequential.
3. Backward compatibility remains mandatory: canonical naming introduced via aliases first, removals deferred.
4. Mandatory quality gates from `DEVELOPMENT.md` must pass before completion.

## Execution lanes and estimation model
- Lane A (CLI/contract): `cli.rs`, `commands/*`, `run.rs`, `main.rs`, `model.rs`, contract tests.
- Lane B (backend): `backend/hammerspoon.rs`, `backend/applescript.rs`, `backend/mod.rs`, `preflight.rs`.
- Lane C (docs/completion/test hardening): `README.md`, shell completions, scenario/docs/testing glue.

Estimate mapping (used below):
- Complexity 4-5: 0.5-1.0 engineer-day.
- Complexity 6-7: 1.0-2.0 engineer-days.
- Complexity 8: 2.0-3.0 engineer-days.
- Complexity 9: 3.0-4.0 engineer-days.

## Sprint 1: Contract and output foundation (Week 1)
**Goal**: Ship shared output helpers and unified mutating envelope without breaking existing behavior.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test cli_smoke`
- Verify:
  - Output/error handling is centralized and consistent.
  - Mutating commands expose uniform `policy` and `meta` contract.

**Parallelization notes**:
- `Task 1.1` (Lane A) and `Task 1.2` (Lane B/A integration) run in parallel.
- `Task 1.3` is integration and starts only after 1.1 + 1.2.

### Task 1.1: Centralize command emit/reject helpers
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
- **Description**: Replace repeated JSON serialization and TSV rejection branches with shared helper functions. Lane: A. Estimated effort: 1.5-2.0 days.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Shared helper is used across command handlers.
  - Command payload shape and behavior remain backward compatible.
  - Error messages remain operation-aware.
- **Validation**:
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test cli_smoke`

### Task 1.2: Unify mutating envelope for AX mutators missing policy/meta
- **Location**:
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/commands/ax_attr.rs`
  - `crates/macos-agent/src/commands/ax_action.rs`
  - `crates/macos-agent/src/commands/ax_session.rs`
  - `crates/macos-agent/src/commands/ax_watch.rs`
- **Description**: Add consistent `policy` and `meta` payload blocks to mutating AX commands that currently diverge. Lane: B with A review. Estimated effort: 2.0-3.0 days.
- **Dependencies**:
  - none
- **Complexity**: 8
- **Acceptance criteria**:
  - All mutating command responses follow one documented contract.
  - Dry-run responses preserve contract shape.
  - Existing JSON consumers can parse one common envelope.
- **Validation**:
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test scenario_chain`

### Task 1.3: Consolidate command identity mapping (dispatch + trace)
- **Location**:
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/main.rs`
  - `crates/macos-agent/tests/cli_smoke.rs`
- **Description**: Introduce single source for command labels used by trace and runtime identity to prevent drift. Lane: A. Estimated effort: 1.0 day.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Label mapping changes in one place.
  - Trace and runtime command identity are consistent.
- **Validation**:
  - `cargo test -p macos-agent --test cli_smoke`
  - `cargo test -p macos-agent --test contracts`

## Sprint 2: UX normalization and discoverability (Week 2)
**Goal**: Make command usage more intuitive via shared AX args, canonical naming, and explicit decision guidance.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --lib cli::tests`
  - `zsh -f tests/zsh/completion.test.zsh`
- Verify:
  - AX selectors/targets are centrally defined.
  - Canonical naming is visible in help/completions while aliases preserve compatibility.

**Parallelization notes**:
- `Task 2.1` (Lane A) and `Task 2.2` (Lane C+A) run in parallel.
- `Task 2.3` starts after 2.1 + 2.2.

### Task 2.1: Flatten reusable AX target/selector args
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
- **Description**: Introduce shared `clap` fragments and centralized validation for AX selector/target constraints. Lane: A. Estimated effort: 1.5-2.0 days.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - AX arg definitions are reusable and not duplicated.
  - Selector/target error semantics are consistent across AX subcommands.
- **Validation**:
  - `cargo test -p macos-agent --lib cli::tests`
  - `cargo test -p macos-agent --test cli_smoke`

### Task 2.2: Canonical flag naming + alias migration layer
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/commands/input_type.rs`
  - `crates/macos-agent/src/commands/wait.rs`
  - `crates/macos-agent/src/commands/observe.rs`
  - `completions/zsh/_macos-agent`
  - `completions/bash/macos-agent`
- **Description**: Define canonical names for overlapping concepts and keep old forms as aliases. Lane: C with A review. Estimated effort: 2.0-2.5 days.
- **Dependencies**:
  - none
- **Complexity**: 8
- **Acceptance criteria**:
  - Help text and completions prioritize canonical names.
  - Legacy names remain functional through alias compatibility.
  - Behavior parity holds between canonical and alias paths.
- **Validation**:
  - `cargo test -p macos-agent --lib cli::tests`
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "window_title_contains|window-name|window_title" completions/bash/macos-agent`

### Task 2.3: Add explicit user decision guidance in docs/help
- **Location**:
  - `crates/macos-agent/README.md`
  - `crates/macos-agent/src/cli.rs`
- **Description**: Add AX-first vs fallback decision path with clear examples and troubleshooting entry points. Lane: C. Estimated effort: 0.5-1.0 day.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Users can choose `ax` vs `input` vs `wait` quickly from docs/help.
  - Guidance remains consistent with actual command behavior.
- **Validation**:
  - `rg -n "ax-first|fallback|input\\.|decision" crates/macos-agent/README.md`
  - `cargo run -p macos-agent -- --help | rg -i "ax|fallback|input"`
  - `cargo run -p macos-agent -- --help`

## Sprint 3: Backend dedup and capability transparency (Weeks 3-4)
**Goal**: Reduce backend maintenance overhead while making backend capability limits explicit and debuggable.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --lib backend::tests`
  - `cargo test -p macos-agent --test ax_extended`
- Verify:
  - Repeated backend helper logic is centralized.
  - Backend capability constraints are visible to users before failure surprises.

**Parallelization notes**:
- In 2-engineer mode, `Task 3.1` and `Task 3.2` can run concurrently with file partitioning.
- In 1-engineer mode, do `3.2` first (lower risk), then `3.1`, then `3.3`.

### Task 3.1: Deduplicate Hammerspoon helper prelude blocks
- **Location**:
  - `crates/macos-agent/src/backend/hammerspoon.rs`
  - `crates/macos-agent/src/backend/mod.rs`
- **Description**: Build shared Lua prelude fragments and operation-specific assembly to remove repeated helper definitions. Lane: B. Estimated effort: 3.0-4.0 days.
- **Dependencies**:
  - none
- **Complexity**: 9
- **Acceptance criteria**:
  - Shared helpers are not copied per operation.
  - AX behavior parity remains intact under existing tests.
- **Validation**:
  - `cargo test -p macos-agent --lib backend::tests`
  - `cargo test -p macos-agent --test ax_extended`

### Task 3.2: Remove duplicated JXA helper definitions and tighten parser checks
- **Location**:
  - `crates/macos-agent/src/backend/applescript.rs`
  - `crates/macos-agent/src/backend/mod.rs`
- **Description**: Eliminate duplicated JXA functions and improve JSON parse/contract guards. Lane: B. Estimated effort: 1.5-2.0 days.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - Duplicate helper definitions are removed.
  - Parse failures remain operation-specific and actionable.
- **Validation**:
  - `cargo test -p macos-agent --lib backend::tests`
  - `cargo test -p macos-agent --test ax_extended`

### Task 3.3: Surface backend capability matrix in diagnostics and preflight
- **Location**:
  - `crates/macos-agent/src/backend/mod.rs`
  - `crates/macos-agent/src/preflight.rs`
  - `crates/macos-agent/src/error.rs`
  - `crates/macos-agent/README.md`
- **Description**: Document and expose capability boundaries for `auto/hammerspoon/applescript`, especially Hammerspoon-only AX features. Lane: A+C. Estimated effort: 1.0-1.5 days.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Unsupported backend paths provide capability-aware hints.
  - Preflight/docs communicate readiness and likely fallback behavior.
- **Validation**:
  - `cargo test -p macos-agent --test preflight`
  - `cargo test -p macos-agent --test contracts`

## Sprint 4: Stability hardening and rollout gate (Week 5)
**Goal**: Increase confidence with compatibility-focused coverage and ship a rollback-safe handoff.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --test cli_smoke`
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test scenario_chain`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify:
  - Alias compatibility and unified contract are enforced by tests.
  - Docs/completions/release notes align with final behavior.

**Parallelization notes**:
- `Task 4.1` and `Task 4.2` parallel.
- `Task 4.3` final integration gate.

### Task 4.1: Add compatibility and regression tests
- **Location**:
  - `crates/macos-agent/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/tests/scenario_chain.rs`
  - `tests/zsh/completion.test.zsh`
- **Description**: Add assertions for alias/canonical parity, unified mutating envelope, and fallback telemetry stability. Lane: A+C. Estimated effort: 2.0-2.5 days.
- **Dependencies**:
  - Task 1.2
  - Task 2.2
  - Task 3.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Alias and canonical paths are behaviorally equivalent.
  - Contracts enforce unified schema and telemetry semantics.
- **Validation**:
  - `cargo test -p macos-agent --test cli_smoke`
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p macos-agent --test scenario_chain`
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.2: Publish user matrix and migration guidance
- **Location**:
  - `crates/macos-agent/README.md`
  - `docs/plans/macos-agent-realtime-feedback-usability-hardening-plan.md`
- **Description**: Publish command decision matrix and migration notes for canonical naming with alias window. Lane: C. Estimated effort: 1.0 day.
- **Dependencies**:
  - Task 2.3
  - Task 3.3
- **Complexity**: 5
- **Acceptance criteria**:
  - README includes matrix + migration notes + troubleshooting linkage.
- **Validation**:
  - `rg -n "decision matrix|migration|alias|fallback|backend" crates/macos-agent/README.md`

### Task 4.3: Run mandatory gate + release handoff
- **Location**:
  - `DEVELOPMENT.md`
  - `crates/macos-agent/README.md`
  - `crates/macos-agent/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/tests/scenario_chain.rs`
  - `crates/macos-agent/tests/ax_extended.rs`
  - `crates/macos-agent/tests/preflight.rs`
- **Description**: Execute full required checks, write residual-risk summary, and finalize rollback trigger checklist for rollout. Lane: A+B+C final pass. Estimated effort: 1.0-1.5 days.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Mandatory checks pass (or failures are explicitly documented with remediation).
  - Release handoff includes risk and rollback trigger list.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

## Timeline and critical path
- 2-engineer model (recommended): ~5 weeks calendar time.
  - Critical path: `1.2 -> 1.3 -> 2.2 -> 2.3 -> 3.1 -> 3.3 -> 4.1 -> 4.3`.
- 1-engineer model (fallback): ~7-8 weeks calendar time.
  - Execute strictly by dependency order, no parallel lanes.

## Daily execution rhythm
- Daily start:
  - Pick highest-priority unblocked task on critical path.
  - Reconfirm local baseline with task-specific validation command.
- Daily end:
  - Run task validation commands.
  - Record pass/fail and unresolved risks in PR notes.
- Sprint close:
  - Run sprint demo commands.
  - Apply Go/No-Go gate before moving forward.

## Go/No-Go gates
- Sprint 1 gate:
  - `contracts` and `cli_smoke` green, no schema drift for existing consumers.
- Sprint 2 gate:
  - CLI alias compatibility confirmed by tests and completion checks.
- Sprint 3 gate:
  - Backend dedup tests green and capability diagnostics clear.
- Sprint 4 gate:
  - Full required checks pass and release handoff complete.

## Testing Strategy
- Unit:
  - `cli::tests` for parsing/alias/selector normalization.
  - backend parser and capability-path unit tests.
- Integration:
  - `contracts`, `scenario_chain`, `cli_smoke` for end-to-end command contract stability.
- E2E/manual:
  - AX-first local smoke flow plus fallback path verification using documented examples.

## Risks & gotchas
- Large file overlap risk (`cli.rs`, `backend/hammerspoon.rs`) can create merge conflicts if lanes are not strictly scoped.
- Alias migration risk if help/completion/docs are not updated in same sprint as parsing changes.
- Backend dedup risk can cause subtle selector regressions without targeted AX regression tests.

## Rollback plan
- Keep one PR per task or tightly related pair to allow surgical revert.
- For compatibility regressions, keep aliases active and revert canonical-help promotion first.
- For backend regressions, revert `hammerspoon.rs`/`applescript.rs` dedup commits independently while preserving CLI/documentation gains.
- Trigger rollback when any of the following occurs:
  - `contracts` or `scenario_chain` regressions.
  - Reproducible AX behavior drift from pre-refactor baseline.
  - External user script breakage on legacy flags.
