# Plan: macos-agent AX subcommands MVP (ax list / ax click / ax type)

## Overview
This plan adds an Accessibility-first command surface to `macos-agent` so automation can target semantic UI elements before falling back to coordinates. The MVP introduces three new commands: `ax list`, `ax click`, and `ax type`, implemented with a deterministic backend contract and machine-parseable output. Existing `input click/type` and `window activate/wait` behaviors remain unchanged and become fallback primitives for cases where AX actions are unavailable. The implementation prioritizes safe rollout: read-path first (`ax list`), then mutating actions with strict selector rules, and finally docs/completion parity.

## Scope
- In scope:
  - New CLI command group `ax` with subcommands `list`, `click`, and `type`.
  - AX selector contract and stable JSON result schema for agent workflows.
  - Minimal AX backend adapter over `osascript`/JXA (plus fallback bridge to existing `input` flows when explicitly required).
  - Unit and integration coverage for command parsing, selector resolution, and output/error contracts.
  - Completion updates (`zsh` and `bash`) and README command documentation.
- Out of scope:
  - OCR/CV element recognition.
  - Cross-platform AX support (non-macOS).
  - Full workflow DSL changes beyond the existing `scenario run` command.
  - Multi-window persistent element handles across process restarts.

## Assumptions (if any)
1. macOS hosts grant Accessibility and Automation permissions to the calling terminal process.
2. AX tree reads are performed via `osascript` with JXA payloads and can return JSON within current timeout policy bounds.
3. Element references are ephemeral; selectors must be resolved at action time.
4. Existing `input` and `wait` commands remain the fallback path for non-pressable or non-focusable elements.

## AX command spec (MVP)
- `ax list`
  - Purpose: enumerate AX nodes for a target app/window scope.
  - Selectors: `--app <name>` or `--bundle-id <id>` (default frontmost app when omitted).
  - Filters: `--role <AXRole>`, `--title-contains <text>`, `--max-depth <n>`, `--limit <n>`.
  - Output fields (JSON): `node_id`, `role`, `subrole`, `title`, `identifier`, `value_preview`, `enabled`, `focused`, `frame`, `actions`, `path`.
- `ax click`
  - Purpose: resolve exactly one AX node and perform press/click semantics.
  - Selector priority: `--node-id` (from `ax list`) or compound selector (`--role`, `--title-contains`, optional `--nth`).
  - Behavior: execute AX press when available; optional fallback to center-point click only when `--allow-coordinate-fallback` is set.
  - Failure policy: zero-match or multi-match is deterministic runtime error with top candidate hints.
- `ax type`
  - Purpose: focus/set value on target AX element and type/paste text.
  - Required args: selector + `--text`.
  - Options: `--clear-first`, `--submit` (press Enter after typing), `--paste` (clipboard paste strategy).
  - Behavior: prefer AX value set/focus path; fallback to `input type` only when `--allow-keyboard-fallback` is set.

## MVP slice boundaries
- Slice A (read-only): `ax list` end-to-end with filters and JSON contract.
- Slice B (single action): `ax click` with strict selector resolution and optional coordinate fallback.
- Slice C (text action): `ax type` with focus/value strategy and optional keyboard fallback.
- Slice D (integration polish): scenario compatibility, completions, docs, and diagnostics.

## Sprint 1: Contract and AX backend foundation
**Goal**: Lock CLI and model contracts for AX commands and build a deterministic backend adapter for AX tree query.
**Demo/Validation**:
- Command(s):
  - `cargo run -p macos-agent -- --help`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test cli_smoke`
- Verify:
  - `ax` command group appears in help and parses all MVP flags.
  - AX backend interface is isolated from command handlers and emits parseable domain results.

**Parallelization notes**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.3` depends on both and finalizes integration.

### Task 1.1: Add CLI command tree and argument models for `ax`
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/main.rs`
  - `crates/macos-agent/tests/cli_smoke.rs`
- **Description**: Introduce `Ax` command group with `list`, `click`, and `type` subcommands, selector arguments, fallback flags, and format constraints aligned with existing command behaviors.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Help output lists `ax` and all three subcommands.
  - `clap` parsing rejects invalid selector combinations deterministically.
  - `command_label` and tracing logic include `ax.list`, `ax.click`, and `ax.type`.
- **Validation**:
  - `cargo run -p macos-agent -- --help`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test cli_smoke -- help_lists_command_groups`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- trace_command_labels_include_ax_commands`

### Task 1.2: Define AX models and backend adapter contract
- **Location**:
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/src/backend/mod.rs`
  - `crates/macos-agent/src/backend/applescript.rs`
  - `crates/macos-agent/src/error.rs`
- **Description**: Add `AxNode`, selector/result structs, and adapter functions for AX tree query and AX action invocation via `osascript`/JXA with stable error mapping and hints.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - AX domain types serialize under `schema_version=1` JSON envelope.
  - Backend surfaces operation-specific errors (`ax.list`, `ax.click`, `ax.type`).
  - Timeout and parse failures include actionable remediation hints.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- error_format_json_emits_machine_parseable_payload`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test preflight -- preflight_json_structure_is_deterministic`

### Task 1.3: Add command handlers for `ax list` with deterministic output
- **Location**:
  - `crates/macos-agent/src/commands/mod.rs`
  - `crates/macos-agent/src/commands/ax_list.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/tests/list_commands.rs`
  - `crates/macos-agent/tests/contracts.rs`
- **Description**: Implement `ax list` using AX backend query, deterministic ordering, depth/limit filters, and text/json output parity with existing contracts.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - `ax list --format json` returns node list with required fields.
  - `--max-depth` and `--limit` are enforced predictably.
  - Stdout/stderr discipline matches existing command contract tests.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test list_commands`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- success_commands_write_stdout_only`

## Sprint 2: Mutating AX actions (`ax click`, `ax type`)
**Goal**: Ship safe AX mutating commands with strict selector semantics and explicit fallback policy.
**Demo/Validation**:
- Command(s):
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test input_click -- --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test input_keyboard -- --nocapture`
- Verify:
  - `ax click` and `ax type` enforce selector uniqueness and safe fallback behavior.
  - Retry/timeout metadata is preserved in command results and traces.

**Parallelization notes**:
- `Task 2.1` and `Task 2.2` can run in parallel after Sprint 1.
- `Task 2.3` depends on both for final fallback-policy integration.

### Task 2.1: Implement `ax click` command and selector resolution
- **Location**:
  - `crates/macos-agent/src/commands/ax_click.rs`
  - `crates/macos-agent/src/commands/mod.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/retry.rs`
  - `crates/macos-agent/tests/input_click.rs`
- **Description**: Resolve a single AX node by node id or compound selector, perform AX press, and return parseable action metadata. Add candidate hint output for ambiguous selection.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Multi-match and zero-match produce deterministic runtime errors with hints.
  - `--allow-coordinate-fallback` gates fallback click behavior explicitly.
  - JSON output includes action policy and attempts metadata.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test input_click -- --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- error_commands_write_stderr_only_with_error_prefix`

### Task 2.2: Implement `ax type` command with focus/value strategy
- **Location**:
  - `crates/macos-agent/src/commands/ax_type.rs`
  - `crates/macos-agent/src/backend/applescript.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/tests/input_keyboard.rs`
- **Description**: Add `ax type` flow that resolves target node, optionally clears current value, applies text via AX value set or focused keystroke path, and optionally submits Enter.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - `--text` is mandatory and empty text is rejected with usage error.
  - `--clear-first`, `--submit`, and `--paste` behaviors are deterministic.
  - `--allow-keyboard-fallback` is opt-in and clearly surfaced in output.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test input_keyboard -- --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- error_format_json_emits_machine_parseable_payload`

### Task 2.3: Wire scenario compatibility and fallback diagnostics
- **Location**:
  - `crates/macos-agent/src/commands/scenario.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/tests/scenario_chain.rs`
  - `crates/macos-agent/tests/contracts.rs`
- **Description**: Extend scenario step parser/executor to support new AX steps and include fallback usage markers in step telemetry for debugging flaky paths.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Scenario files can invoke `ax.list`, `ax.click`, and `ax.type`.
  - Step results indicate whether AX-native path or fallback path executed.
  - Failure traces include command-level operation names for AX actions.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test scenario_chain -- --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- trace_writes_artifacts_for_success_and_failure`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test scenario_chain -- scenario_steps_report_ax_path_and_fallback_state --nocapture`

## Sprint 3: Completion parity, docs, and delivery hardening
**Goal**: Make AX commands discoverable, documented, and release-ready with full required checks.
**Demo/Validation**:
- Command(s):
  - `zsh -f tests/zsh/completion.test.zsh`
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - Shell completions expose AX commands and options.
  - Workspace-wide formatting/lint/tests continue passing.

**Parallelization notes**:
- `Task 3.1` and `Task 3.2` can run in parallel.
- `Task 3.3` depends on both and is the final gate.

### Task 3.1: Update completion scripts for AX command family
- **Location**:
  - `completions/zsh/_macos-agent`
  - `completions/bash/macos-agent`
  - `tests/zsh/completion.test.zsh`
- **Description**: Add `ax` command completion entries and flag completions for list/click/type selectors and fallback toggles, matching CLI constraints.
- **Dependencies**:
  - Task 1.1
  - Task 2.1
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Completion suggests `ax` at root and each subcommand at level 2.
  - Option completion aligns with CLI flags and enumerated values.
  - Existing completion behavior for other commands remains unchanged.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 3.2: Document AX commands, constraints, and troubleshooting
- **Location**:
  - `crates/macos-agent/README.md`
  - `docs/plans/macos-agent-realtime-feedback-usability-hardening-plan.md`
- **Description**: Add README command reference examples, fallback semantics, and troubleshooting guidance for AX permission gaps, selector ambiguity, and app-specific quirks.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 3
- **Acceptance criteria**:
  - README includes runnable examples for `ax list`, `ax click`, and `ax type`.
  - Troubleshooting section clearly differentiates AX failures from coordinate fallback failures.
  - Plan cross-reference documents follow-up hardening work beyond MVP.
- **Validation**:
  - `rg -n "ax list|ax click|ax type|fallback|Accessibility" crates/macos-agent/README.md`

### Task 3.3: Execute mandatory pre-delivery checks and package release notes
- **Location**:
  - `DEVELOPMENT.md`
  - `crates/macos-agent/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/tests/input_click.rs`
  - `crates/macos-agent/tests/input_keyboard.rs`
  - `crates/macos-agent/tests/list_commands.rs`
  - `crates/macos-agent/tests/scenario_chain.rs`
- **Description**: Run full required checks from `DEVELOPMENT.md`, summarize AX command behavior changes and residual risks for release notes/PR body.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Required checks (`fmt`, `clippy`, workspace tests, zsh completion tests) pass.
  - PR notes include rollout cautions for AX permission and selector drift.
  - Any non-run checks are explicitly documented with reason.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit:
  - Selector parsing, uniqueness validation, and fallback-flag gating.
  - AX backend payload parsing and deterministic ordering.
- Integration:
  - Command contract tests for stdout/stderr discipline and JSON envelopes.
  - Dry-run and trace coverage for `ax click/type` action metadata.
- E2E/manual:
  - Local macOS opt-in checks using `MACOS_AGENT_REAL_E2E=1` with Finder as baseline app.

## Risks & gotchas
- AX trees vary by app version and runtime state; selectors may drift.
- Some apps expose partial AX actions, requiring opt-in fallback.
- Deep tree traversal can be slow; depth and limit bounds are required to keep response latency stable.
- Text input remains vulnerable to IME/layout differences when keyboard fallback is enabled.

## Rollback plan
- Feature-gate AX command group behind compile-time module export and remove `ax` dispatch wiring if release regression is detected.
- Keep existing `input` and `window/wait` command paths unchanged so operational workflows continue without AX.
- Revert completion/docs updates together with command removal to avoid stale user-facing surface.
- If only mutating AX paths are unstable, retain `ax list` and temporarily disable `ax click/type` handlers with clear runtime messages.
