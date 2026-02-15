# Plan: macos-agent primitive reliability and debug upgrades for Codex-driven automation

## Overview
This plan upgrades `macos-agent` primitives so Codex can automate unstable, dynamic macOS UIs with higher determinism and better failure diagnostics. The focus is to add missing guardrails in the core command layer: element wait primitives, richer selector matching, action context gating, postcondition verification, robust click re-selection, and first-class debug/observation tooling. The implementation keeps browser-specific semantics out of core primitives and instead provides generic contracts that can be composed by wrappers. Delivery is staged to preserve backward compatibility while improving reliability for all app types, not only browser workflows.

## Scope
- In scope:
  - Add `wait ax-present` and `wait ax-unique` primitives for selector readiness checks.
  - Extend selector matching with strategy controls and selector debug explain output.
  - Add `ax click`/`ax type` action context gating (`ensure app/window/element ready`) before mutation.
  - Add generic postcondition checks for mutating AX commands.
  - Improve click execution robustness via re-selection and fallback-chain controls.
  - Add a debug bundle command to emit triage artifacts in one run.
  - Extend `observe screenshot` with selector-frame capture support.
- Out of scope:
  - Browser-only semantics in core commands (for example hard-coded URL-specific checks).
  - OCR/computer-vision based element detection.
  - Non-macOS platform support.

## Assumptions (if any)
1. Backward compatibility for existing command defaults remains required, with new behavior introduced by additive flags/subcommands.
2. `macos-agent` keeps JSON/text dual-output contract stability for automation consumers.
3. New diagnostics can write artifacts under `${AGENTS_HOME:-$HOME/.agents}/out` without breaking existing flows.
4. Real desktop E2E checks remain opt-in and local-only; deterministic tests stay the default CI gate.

## Sprint 1: Deterministic element targeting and click resilience
**Goal**: Ensure element targeting is deterministic before action execution and reduce stale-selector failures in dynamic UIs.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --test wait -- --nocapture`
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`
  - `cargo test -p macos-agent --test contracts -- --nocapture`
- Verify:
  - AX element readiness can be explicitly waited on.
  - Selector matching is configurable and diagnosable.
  - Click behavior is resilient against UI re-render and stale node ids.

**Parallelization notes**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.3` depends on both `Task 1.1` and `Task 1.2`.

### Task 1.1: Add `wait ax-present` and `wait ax-unique` primitives
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/commands/wait.rs`
  - `crates/macos-agent/src/commands/ax_common.rs`
  - `crates/macos-agent/src/backend/hammerspoon.rs`
  - `crates/macos-agent/tests/wait.rs`
  - `crates/macos-agent/tests/ax_extended.rs`
- **Description**: Introduce wait subcommands that poll AX selectors until at least one match (`ax-present`) or exactly one match (`ax-unique`) within timeout/poll constraints. Emit structured wait metadata to support deterministic retries in higher-level automation flows.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - New wait subcommands accept selector filters consistent with existing AX commands.
  - Timeout and poll controls behave consistently with existing wait commands.
  - JSON output includes condition, attempts, elapsed time, and terminal status.
- **Validation**:
  - `cargo test -p macos-agent --test wait -- --nocapture`
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`

### Task 1.2: Add selector match strategy and selector explain diagnostics
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/src/commands/ax_common.rs`
  - `crates/macos-agent/src/backend/hammerspoon.rs`
  - `crates/macos-agent/tests/ax_extended.rs`
  - `crates/macos-agent/tests/contracts.rs`
- **Description**: Extend selector filters with explicit match strategy controls (`contains`, `exact`, `prefix`, `suffix`, `regex`) and add a debug explain mode that reports candidate counts after each filter stage. This reduces ambiguous matching and shortens triage time when selectors fail.
- **Dependencies**:
  - none
- **Complexity**: 8
- **Acceptance criteria**:
  - Strategy flags are additive and default to current `contains` behavior.
  - Explain output is available in JSON mode with stable keys for machine parsing.
  - Selector validation errors remain actionable and operation-specific.
- **Validation**:
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`
  - `cargo test -p macos-agent --test contracts -- --nocapture`
  - `cargo test -p macos-agent --test ax_extended -- gating_flags_apply_to_click_and_type --nocapture`
  - `cargo test -p macos-agent --test ax_extended -- gating_flags_disabled_preserve_default_behavior --nocapture`

### Task 1.3: Add click re-selection and fallback-chain controls
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/commands/ax_click.rs`
  - `crates/macos-agent/src/backend/hammerspoon.rs`
  - `crates/macos-agent/src/backend/cliclick.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/tests/ax_extended.rs`
  - `crates/macos-agent/tests/contracts.rs`
- **Description**: Add `ax click` controls for re-resolving selectors before action and for configurable fallback order (AX press/confirm/frame-center/coordinate). This prevents stale node-id failure loops and makes fallback behavior explicit and testable.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Click can re-select target immediately before mutation when requested.
  - Fallback order is configurable and reflected in result metadata.
  - Failure output states which fallback stages were attempted.
- **Validation**:
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`
  - `cargo test -p macos-agent --test contracts -- --nocapture`

## Sprint 2: Action gating and postcondition contract
**Goal**: Guarantee mutating actions run only in a prepared UI context and verify expected state transitions after action execution.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`
  - `cargo test -p macos-agent --test scenario_chain -- --nocapture`
  - `cargo test -p macos-agent --test contracts -- --nocapture`
- Verify:
  - `ax click`/`ax type` can ensure app/window/element readiness before mutation.
  - Mutating actions can assert generic postconditions with timeout-aware polling.

**Parallelization notes**:
- `Task 2.1` starts after `Task 1.1`.
- `Task 2.2` starts after `Task 2.1` and `Task 1.2`.

### Task 2.1: Add action context gating for `ax click` and `ax type`
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/commands/ax_click.rs`
  - `crates/macos-agent/src/commands/ax_type.rs`
  - `crates/macos-agent/src/commands/window_activate.rs`
  - `crates/macos-agent/src/commands/wait.rs`
  - `crates/macos-agent/src/commands/ax_common.rs`
  - `crates/macos-agent/tests/ax_extended.rs`
- **Description**: Add optional pre-action gates to ensure app active, target window present, and selector readiness before mutation. Integrate gate execution into command flow so wrappers no longer need to chain multiple preconditions manually.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Gating flags are available to both `ax click` and `ax type`.
  - Gate failures return actionable context instead of generic selector-not-found errors.
  - Existing default behavior remains unchanged when gating flags are omitted.
- **Validation**:
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`
  - `cargo test -p macos-agent --test contracts -- --nocapture`

### Task 2.2: Add generic postcondition checks for mutating AX commands
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/commands/ax_click.rs`
  - `crates/macos-agent/src/commands/ax_type.rs`
  - `crates/macos-agent/src/commands/ax_attr.rs`
  - `crates/macos-agent/src/backend/hammerspoon.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/tests/ax_extended.rs`
  - `crates/macos-agent/tests/scenario_chain.rs`
- **Description**: Introduce postcondition primitives for mutating AX commands (for example expected attribute/value/title/focus conditions with timeout and poll controls). This creates a general verification contract usable by browser and non-browser workflows.
- **Dependencies**:
  - Task 1.2
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Postcondition checks support at least attribute-value and focus-state verification.
  - Result payload includes postcondition evaluation metadata and elapsed wait.
  - Failure reason clearly distinguishes action failure vs postcondition mismatch.
- **Validation**:
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`
  - `cargo test -p macos-agent --test scenario_chain -- --nocapture`

## Sprint 3: Unified debug bundle and element-aware screenshoting
**Goal**: Provide first-class triage artifacts and element-level visual inspection primitives for fast, deterministic debugging.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --test contracts -- --nocapture`
  - `cargo test -p macos-agent --test preflight -- --nocapture`
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`
- Verify:
  - One command can generate a complete debug bundle for failed runs.
  - Observe command can capture selector-frame screenshots with metadata.

**Parallelization notes**:
- `Task 3.1` and `Task 3.2` can run in parallel after `Task 1.2`.
- If artifact schema overlap becomes risky, do `Task 3.1` first then align `Task 3.2`.

### Task 3.1: Add `debug bundle` command for one-shot triage artifacts
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/commands/observe.rs`
  - `crates/macos-agent/src/commands/list.rs`
  - `crates/macos-agent/src/commands/ax_list.rs`
  - `crates/macos-agent/src/commands/ax_common.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/README.md`
- **Description**: Add a command that captures a standard diagnostic bundle in one invocation: active-window screenshot, window list, AX candidate lists (`AXLink`/`AXButton`/`AXTextField`), focused element snapshot, and current target metadata. Implement in two sequential slices within this task: first command/schema output and partial-failure reporting, then README triage-flow documentation. Standardize artifact naming and index output.
- **Dependencies**:
  - Task 1.2
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Debug bundle writes deterministic artifact index entries in JSON mode.
  - Bundle capture gracefully handles partial failures and reports missing pieces.
  - README includes copy-paste triage flow using debug bundle output.
- **Validation**:
  - `cargo test -p macos-agent --test contracts -- --nocapture`
  - `cargo test -p macos-agent --test preflight -- --nocapture`
  - `cargo test -p macos-agent --test contracts -- debug_bundle_emits_artifact_index_and_partial_failure_entries --nocapture`
  - `rg -n "debug bundle|triage flow|artifact index" crates/macos-agent/README.md`

### Task 3.2: Extend `observe screenshot` to support AX selector frame capture
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/commands/observe.rs`
  - `crates/macos-agent/src/commands/ax_common.rs`
  - `crates/macos-agent/src/backend/hammerspoon.rs`
  - `crates/macos-agent/src/targets.rs`
  - `crates/macos-agent/src/screen_record_adapter.rs`
  - `crates/macos-agent/tests/ax_extended.rs`
  - `crates/macos-agent/tests/contracts.rs`
- **Description**: Add selector-driven element frame screenshot support with optional padding and metadata output. This enables visual verification of exact targeted elements instead of only full-window captures.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Observe command can resolve selector to frame and capture a bounded screenshot region.
  - Output includes frame coordinates, selector summary, and final artifact path.
  - Full-window screenshot behavior remains backward compatible.
- **Validation**:
  - `cargo test -p macos-agent --test ax_extended -- --nocapture`
  - `cargo test -p macos-agent --test contracts -- --nocapture`

## Testing Strategy
- Unit:
  - Validate selector strategy parsing, matching behavior, and explain payload schema.
  - Validate postcondition evaluator logic and timeout behavior.
- Integration:
  - Exercise command-level contracts for `wait`, `ax click/type`, `observe`, and new debug bundle outputs.
  - Ensure JSON/text outputs remain stable where backward compatibility is required.
- E2E/manual:
  - Run opt-in real desktop checks for dynamic apps (Arc/Chrome/Finder/Spotify) using new gating and debug commands.
  - Confirm stale selector and re-render scenarios are diagnosable with artifact bundles.
- Regression guard:
  - Keep existing deterministic suites as default CI gate and add targeted new tests per primitive.

## Risks & gotchas
- Adding multiple new flags can increase CLI complexity unless help text and examples are explicit.
- Selector strategy misuse (for example broad regex) can increase false positives without strict validation.
- Debug bundle artifact volume can grow quickly on repeated retries; retention guidance is needed.
- Frame-capture logic may differ across display scale factors and multi-monitor setups.

## Rollback plan
- Keep all new primitive behaviors additive and opt-in by default in the first rollout phase.
- If instability appears, disable new gating/postcondition/debug commands behind feature toggles while preserving existing command paths.
- Revert selector strategy extensions to default `contains` behavior and retain compatibility aliases.
- Maintain existing full-window observe path as safe fallback if element-frame capture is unstable.
- Use deterministic contract tests as release blocker before re-enabling new primitives.
