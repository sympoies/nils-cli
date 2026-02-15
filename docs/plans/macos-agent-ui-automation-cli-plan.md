# Plan: macOS agent UI automation CLI (AppleScript + Accessibility + cliclick)

## Overview
This plan introduces a new CLI, `macos-agent`, so agents can operate macOS desktop apps with
scriptable primitives: click, type, shortcut, and window switching. Phase 1 is macOS-only and
prioritizes reliability and parseable output over broad feature coverage. The plan explicitly
evaluates and reuses `crates/screen-record` for window discovery, selection, and screenshot capture
to avoid duplicating fragile macOS integration logic.

## Scope
- In scope:
  - New workspace crate and binary: `crates/macos-agent` -> `macos-agent`.
  - macOS-only command surface for:
    - environment preflight checks
    - listing windows and apps
    - activating app/window context
    - pointer click actions
    - keyboard typing and shortcut actions
    - optional screenshot observation for verification artifacts
  - Reuse of `screen-record` modules where practical (`types`, selection logic, shareable content,
    screenshot capture).
  - Parseable output contract for agent workflows (machine-readable mode + stable exit codes).
  - Deterministic test mode and CI-safe tests that do not require real desktop control.
- Out of scope:
  - Linux or Windows support.
  - OCR, CV-based element detection, or semantic UI understanding.
  - Full RPA workflow engine (loops, branching DSL, long-lived daemon).
  - Human-facing GUI; this is CLI-first automation.
  - Hard real-time guarantees across all apps and animation-heavy UIs.

## Assumptions (if any)
1. Target runtime is macOS 13+ for primary validation, with best effort support for macOS 12+.
2. `osascript` is available on macOS by default; `cliclick` is an external dependency installed by user.
3. Users grant Accessibility and Automation permissions to the terminal host running `macos-agent`.
4. Screen recording permission is needed only when screenshot observation is used.
5. Desktop UI automation is inherently brittle; v1 mitigates this with retries, waits, and explicit errors.

## Sprint 1: Contract, feasibility, and crate scaffold
**Goal**: Lock command contract and architecture, then create a buildable crate with preflight checks.
**Demo/Validation**:
- Command(s):
  - `cargo run -p macos-agent -- --help`
  - `cargo run -p macos-agent -- preflight --format json`
- Verify:
  - CLI shows planned subcommands and global flags.
  - Preflight reports dependency and permission status in a parseable schema.

**Parallelization notes**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.4` starts after `Task 1.3`.

### Task 1.1: Define command surface and output contract
- **Location**:
  - `crates/macos-agent/README.md`
- **Description**: Write the product contract for `macos-agent`, including subcommands, arguments,
  stable stdout and stderr behavior, and exit code mapping. Define response schema for
  machine-readable mode and include concrete examples for click, type, hotkey, and window switching.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - README documents at least these command groups: `preflight`, `windows`, `apps`, `window`,
    `input`, and `observe`.
  - README defines exit code semantics (`0` success, `1` runtime failure, `2` usage error).
  - README examples avoid prose-only outputs and show parseable command results.
- **Validation**:
  - `rg -n "preflight|windows|apps|window activate|input click|input type|input hotkey|observe screenshot" crates/macos-agent/README.md`

### Task 1.2: Evaluate screen-record reuse and lock architecture decision
- **Location**:
  - `docs/plans/macos-agent-ui-automation-cli-plan.md`
  - `crates/screen-record/src/lib.rs`
  - `crates/screen-record/src/macos/mod.rs`
- **Description**: Produce a reuse matrix that evaluates direct reuse vs extraction for
  `screen-record` modules (`types`, `select`, `macos::shareable`, `macos::screenshot`,
  permission helpers). Select one concrete approach for v1 and define fallback if API coupling
  becomes unstable.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Plan includes explicit keep/reuse/do-not-reuse decisions per module.
  - Plan documents coupling risks, CI implications, and migration path if extraction is needed later.
  - v1 decision is actionable without requiring an additional spike sprint.
- **Validation**:
  - `rg -n "Task 1.2 architecture decision|Reuse matrix|Decision|Coupling risks|CI implications|Migration path" docs/plans/macos-agent-ui-automation-cli-plan.md`

#### Task 1.2 architecture decision (merged from ADR on 2026-02-06)
- **Status**: Accepted
- **Decision summary**:
  - For v1, `macos-agent` reuses `screen-record` modules for `types`, `select`,
    `macos::shareable`, and `macos::screenshot`.
  - `macos-agent` does not reuse `screen-record` permission helpers.
  - No extraction spike is required in Sprint 1.

**Reuse matrix**

| Module / area | Current role | v1 decision | Why | Coupling risk and mitigation | CI implications |
| --- | --- | --- | --- | --- | --- |
| `types` | Canonical structs: `WindowInfo`, `DisplayInfo`, `AppInfo`, `ShareableContent` | **Keep + reuse directly** | Stable data model already used by discovery and selection; no unsafe code; lowest-risk reuse | Risk: field changes can ripple into `macos-agent`. Mitigation: consume through a thin adapter layer in `macos-agent` so output schema stays independent. | Cross-platform-safe data types; no additional platform linkage risk. |
| `select` | Deterministic selector resolution (`--window-id`, `--active-window`, `--app`, `--window-name`) and ambiguous candidate rendering | **Keep + reuse with adapter mapping** | Reusing avoids re-implementing ambiguity/frontmost behavior and keeps selector semantics consistent | Risk: tied to `CliError` and error text that mentions `screen-record` style flags. Mitigation: map `CliError` into `macos-agent` errors at adapter boundary; avoid exposing raw error text contract as public API. | Unit tests for selector behavior can run on all platforms (pure Rust). |
| `macos::shareable` | ScreenCaptureKit fetch + conversion into `ShareableContent` | **Keep + reuse directly (macOS-only path)** | Highest-value reuse: avoids duplicating fragile Objective-C callback/runloop integration | Risk: module path/API instability inside `screen-record` internals. Mitigation: isolate calls in one `macos-agent::targets` backend file and keep imports private. | Must gate usage with `#[cfg(target_os = "macos")]`; Linux CI must compile alternate stub path. |
| `macos::screenshot` | Window screenshot via ScreenCaptureKit stream + image encode/fallback | **Keep + reuse with format adapter** | Reuse avoids duplicating complex capture pipeline and encoding behavior | Risk: signature depends on `screen_record::cli::ImageFormat` (CLI-coupled type). Mitigation: keep a local `macos-agent` format enum and convert at one boundary function. | macOS-only runtime behavior; tests should use deterministic test-mode abstractions where possible. |
| Permission helpers (`macos::permissions`) | Screen Recording preflight/request + System Settings opener | **Do not reuse in `macos-agent` v1** | `macos-agent` preflight needs broader checks (Accessibility/Automation/cliclick/osascript), not just Screen Recording | Risk if reused: mixed responsibilities and side effects (`open` System Settings) that do not match `macos-agent` preflight UX. Mitigation: implement dedicated `macos-agent` preflight permission checks; optionally call lower-level APIs directly. | Keeps CI deterministic by avoiding side-effecting helper reuse in generic preflight tests. |

**Decision**

For v1, `macos-agent` reuses `screen-record` modules for `types`, `select`, `macos::shareable`, and
`macos::screenshot`, and does not reuse `screen-record` permission helpers.

No extraction spike is required in Sprint 1.

**Coupling risks**

1. API coupling to internal module paths:
   `macos-agent` directly importing `screen_record::macos::*` can break if files are reorganized.
2. Error-contract coupling:
   `select` emits `CliError` and CLI-oriented text; direct passthrough could leak unstable wording into
   `macos-agent` contract.
3. Type coupling in screenshot path:
   `screenshot_window` currently takes `screen_record::cli::ImageFormat`, which is CLI-domain typed.
4. Platform gating coupling:
   `screen_record::macos` is only available on macOS (or special coverage cfg), so imports must be
   strictly cfg-gated.

**CI implications**

- `macos-agent` must keep all direct `screen_record::macos::*` calls inside `#[cfg(target_os = "macos")]`
  modules/functions so Linux workspace builds stay green.
- Selector logic reuse (`types`/`select`) remains testable in normal cross-platform unit tests.
- Screenshot/shareable integration tests should use deterministic seams (adapter trait + test doubles) for
  non-macOS CI, with macOS-only smoke tests for real capture paths.
- Do not rely on `cfg(coverage)` stubs in `screen-record` as the primary portability mechanism for
  `macos-agent`; treat them as coverage support for `screen-record` itself.

**Migration path (if extraction becomes necessary)**

Extraction trigger conditions:

- `macos-agent` and another crate need the same capture API with repeated adapter glue.
- `screen-record` refactors cause repeated breakage in `macos-agent` imports/contracts.
- `CliError`/`ImageFormat` coupling becomes a blocker for independent command contracts.

Migration steps:

1. Add a stable facade in `screen-record` (for example, `screen_record::capture_api`) that exposes
   capture-focused types and functions without CLI-domain enums/errors.
2. Move pure shared logic (`types`, selection algorithm core) behind that facade while keeping existing
   exports as compatibility wrappers.
3. Update `macos-agent` to consume only the facade.
4. After one release cycle, deprecate old direct module imports and remove compatibility wrappers.

**Actionable v1 implementation (no extra spike)**

1. In Task 2.1, build a `macos-agent` target adapter that reuses `screen_record::types` and
   `screen_record::select` behind local interfaces.
2. In Task 2.3, route screenshot observation through `screen_record::macos::shareable::fetch_shareable`
   and `screen_record::macos::screenshot::screenshot_window`, with local format/error conversion.
3. In Task 1.4, implement `macos-agent` preflight permission checks independently (including Accessibility
   and Automation checks), not by reusing `screen_record::macos::permissions`.

**Consequences**

- Short-term delivery speed improves by reusing proven macOS capture code.
- `macos-agent` keeps command-contract control via adapter boundaries.
- A clear extraction route exists if cross-crate coupling grows.

### Task 1.3: Scaffold new crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/macos-agent/Cargo.toml`
  - `crates/macos-agent/src/main.rs`
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/error.rs`
- **Description**: Add `macos-agent` to the workspace and implement clap parsing with the planned
  subcommand tree. Keep non-macOS behavior explicit and safe by exiting with usage error and clear
  messaging when run on unsupported platforms.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - `cargo build --workspace` succeeds after adding the new crate.
  - `cargo run -p macos-agent -- --help` exits 0 and shows full command tree.
  - Non-macOS runtime path exits 2 with a clear unsupported-platform message.
- **Validation**:
  - `cargo run -p macos-agent -- --help`
  - `cargo test -p macos-agent`

### Task 1.4: Implement preflight checks for tools and permissions
- **Location**:
  - `crates/macos-agent/src/preflight.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/tests/preflight.rs`
- **Description**: Implement `preflight` to validate runtime prerequisites:
  macOS version, `osascript`, `cliclick`, and baseline permission readiness signals. Include
  actionable remediation messages for missing dependency or blocked permission states.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Preflight returns deterministic JSON and text output modes.
  - Missing `cliclick` reports a clear install hint without stack traces.
  - Permission failures include precise System Settings guidance.
- **Validation**:
  - `cargo run -p macos-agent -- preflight --format json`
  - `cargo test -p macos-agent -- preflight`

## Sprint 2: Observation and target resolution with screen-record reuse
**Goal**: Provide robust target discovery and screenshot observation primitives for agents.
**Demo/Validation**:
- Command(s):
  - `cargo run -p macos-agent -- windows list --format tsv`
  - `cargo run -p macos-agent -- observe screenshot --active-window --path ./tmp/macos-agent.png`
- Verify:
  - Window and app listing is deterministic and parseable.
  - Screenshot command writes a valid file and prints the resolved path.

**Parallelization notes**:
- `Task 2.2`, `Task 2.3`, and `Task 2.4` can run in parallel after `Task 2.1`.
- Parallel work in this sprint can conflict in `crates/macos-agent/src/cli.rs` and
  `crates/macos-agent/src/run.rs`; sequence final integration through one owner task.

### Task 2.1: Build target adapter around screen-record shareable and selection logic
- **Location**:
  - `crates/macos-agent/Cargo.toml`
  - `crates/macos-agent/src/targets.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/screen-record/src/lib.rs`
- **Description**: Add a thin adapter layer that reuses `screen-record` window models and selection
  behavior while keeping `macos-agent` command contracts independent. Normalize error text and IDs so
  agent workflows can consume results consistently.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Window resolution supports `--window-id`, `--active-window`, and `--app` with optional `--window-name`.
  - Ambiguous target errors include deterministic candidate rows.
  - Adapter code avoids duplicating full selection logic from `screen-record`.
- **Validation**:
  - `cargo test -p macos-agent -- targets`

### Task 2.2: Implement window and app listing commands
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/commands/list.rs`
  - `crates/macos-agent/tests/list_commands.rs`
- **Description**: Implement `windows list` and `apps list` with `json` and `tsv` output modes. Keep
  sorting and row normalization deterministic to support scripted usage and stable snapshots.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `windows list` and `apps list` output is stable across repeated calls in test mode.
  - JSON output includes a top-level `schema_version` field.
  - TSV fields are tab-safe and newline-safe.
- **Validation**:
  - `cargo run -p macos-agent -- windows list --format tsv`
  - `cargo run -p macos-agent -- apps list --format json | jq -e '.schema_version != null'`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- list_commands`

### Task 2.3: Implement screenshot observation command via screen-record backend
- **Location**:
  - `crates/macos-agent/src/commands/observe.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/tests/observe_screenshot.rs`
- **Description**: Add `observe screenshot` command that resolves a target window and delegates
  capture to `screen-record` macOS screenshot module. Ensure path handling, format selection, and
  stdout contract remain deterministic for agent loops.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Command supports target selectors and explicit output path.
  - Successful run prints exactly one output path line.
  - Failure paths keep stdout empty and return actionable stderr.
- **Validation**:
  - `cargo run -p macos-agent -- observe screenshot --active-window --path ./tmp/macos-agent.png`
  - `cargo test -p macos-agent -- observe_screenshot`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- observe_screenshot`

### Task 2.4: Implement wait primitives for UI stabilization
- **Location**:
  - `crates/macos-agent/src/wait.rs`
  - `crates/macos-agent/src/commands/wait.rs`
  - `crates/macos-agent/tests/wait.rs`
- **Description**: Add wait helpers (`wait sleep`, `wait app-active`, `wait window-present`) with
  timeout and polling controls. These primitives reduce timing flakiness before click or keyboard
  actions.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Wait commands support explicit timeout values and deterministic timeout errors.
  - Poll loops do not spam stdout.
  - Command behavior is testable with stubbed target adapters.
- **Validation**:
  - `cargo test -p macos-agent -- wait`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- wait`

## Sprint 3: Action backend for click, type, shortcut, and window switching
**Goal**: Deliver core manipulation actions through AppleScript and cliclick with safety rails.
**Demo/Validation**:
- Command(s):
  - `cargo run -p macos-agent -- window activate --app Terminal --wait-ms 1500`
  - `cargo run -p macos-agent -- input click --x 200 --y 160`
  - `cargo run -p macos-agent -- input type --text "hello world"`
  - `cargo run -p macos-agent -- input hotkey --mods cmd,shift --key 4`
- Verify:
  - Commands execute expected OS actions or fail with actionable, parseable errors.
  - Dry-run mode emits planned actions without mutating desktop state.

**Parallelization notes**:
- `Task 3.2`, `Task 3.3`, and `Task 3.4` can run in parallel after `Task 3.1` and `Task 2.4`.
- `Task 3.5` integrates shared safety policy after the action commands exist.
- Parallel work in this sprint can conflict in `crates/macos-agent/src/cli.rs`,
  `crates/macos-agent/src/run.rs`, and `crates/macos-agent/src/error.rs`; reserve a final merge pass.

### Task 3.1: Implement process runners and backend error mapping
- **Location**:
  - `crates/macos-agent/src/backend/process.rs`
  - `crates/macos-agent/src/backend/applescript.rs`
  - `crates/macos-agent/src/backend/cliclick.rs`
  - `crates/macos-agent/src/error.rs`
- **Description**: Implement shell execution wrappers with timeout handling, argument escaping, and
  structured error mapping for `osascript` and `cliclick`. Preserve command stderr for diagnostics
  while keeping user-facing messages concise.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Backend maps command-not-found and non-zero exit outcomes to stable error categories.
  - Timeout behavior is deterministic and test-covered.
  - Sensitive inputs are not echoed unredacted in error paths.
- **Validation**:
  - `cargo test -p macos-agent -- backend`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- backend`

### Task 3.2: Implement app or window activation command
- **Location**:
  - `crates/macos-agent/src/commands/window_activate.rs`
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/tests/window_activate.rs`
- **Description**: Add `window activate` to switch context by app name, bundle id, or resolved
  window target. Use AppleScript/System Events and optional wait-until-active check to confirm state.
- **Dependencies**:
  - Task 2.1
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Activation supports both app-level and resolved-window selectors.
  - Command can wait for active confirmation with bounded timeout.
  - Failure output includes the failed selector and suggested fallback selector.
- **Validation**:
  - `cargo run -p macos-agent -- window activate --app Terminal --wait-ms 1500`
  - `cargo test -p macos-agent -- window_activate`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- window_activate`

### Task 3.3: Implement pointer click action via cliclick
- **Location**:
  - `crates/macos-agent/src/commands/input_click.rs`
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/tests/input_click.rs`
- **Description**: Add `input click` command for absolute coordinate click, button selection, and
  optional double click. Include pre and post wait hooks so action chains are less timing-sensitive.
- **Dependencies**:
  - Task 2.4
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Command validates coordinate and button arguments with usage errors on invalid input.
  - Double-click path is explicitly supported and test-covered.
  - Runtime errors from `cliclick` are reported without noisy stack traces.
- **Validation**:
  - `cargo run -p macos-agent -- input click --x 200 --y 160`
  - `cargo test -p macos-agent -- input_click`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- input_click`

### Task 3.4: Implement keyboard typing and hotkey actions
- **Location**:
  - `crates/macos-agent/src/commands/input_type.rs`
  - `crates/macos-agent/src/commands/input_hotkey.rs`
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/tests/input_keyboard.rs`
- **Description**: Add `input type` and `input hotkey` commands backed by AppleScript keystroke
  automation. Support modifier combinations and key token validation, with clear handling for
  unsupported keys.
- **Dependencies**:
  - Task 2.4
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Text input supports whitespace and punctuation reliably.
  - Hotkey command validates modifier sets and fails fast on invalid tokens.
  - Command output remains parseable in both success and failure paths.
- **Validation**:
  - `cargo run -p macos-agent -- input type --text "hello world"`
  - `cargo run -p macos-agent -- input hotkey --mods cmd,shift --key 4`
  - `cargo test -p macos-agent -- input_keyboard`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- input_keyboard`

### Task 3.5: Add safety rails (dry-run, retries, timeout, action IDs)
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/retry.rs`
  - `crates/macos-agent/tests/retry.rs`
- **Description**: Add shared execution policy flags: `--dry-run`, `--retries`, `--retry-delay-ms`,
  and `--timeout-ms`. Emit action IDs and timing metadata to help agents debug flaky automation runs.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Dry-run mode never executes OS automation commands.
  - Retry policy applies consistently across activation, click, and keyboard actions.
  - Timeout and retry metadata are visible in machine-readable output.
- **Validation**:
  - `cargo run -p macos-agent -- input click --x 10 --y 10 --dry-run`
  - `cargo test -p macos-agent -- retry`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- retry`

## Sprint 4: Test hardening, docs, completions, and release readiness
**Goal**: Make the CLI CI-safe, well-documented, and ready for workspace-quality checks.
**Demo/Validation**:
- Command(s):
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent`
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - Full crate test suite runs without controlling the real desktop.
  - Workspace checks and coverage gate continue to pass.

**Parallelization notes**:
- `Task 4.2` and `Task 4.3` can run in parallel after `Task 4.1`.
- `Task 4.4` runs after both test and docs/completion work finish.

### Task 4.1: Add deterministic test mode and stubbed OS automation binaries
- **Location**:
  - `crates/macos-agent/src/test_mode.rs`
  - `crates/macos-agent/tests/common.rs`
  - `crates/macos-agent/tests/fixtures/stub-osascript-ok.txt`
  - `crates/macos-agent/tests/fixtures/stub-cliclick-ok.txt`
- **Description**: Implement `AGENTS_MACOS_AGENT_TEST_MODE=1` so tests can exercise command flow
  using stubbed `osascript` and `cliclick` binaries instead of the real desktop. Reuse
  `nils-test-support` helpers for isolated PATH and environment mutation.
- **Dependencies**:
  - Task 3.5
- **Complexity**: 8
- **Acceptance criteria**:
  - Core command tests pass on CI without macOS desktop permissions.
  - Stub outputs cover success, non-zero, and timeout scenarios.
  - Tests remain deterministic across repeated runs.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent`

### Task 4.2: Add integration tests for command contracts and error semantics
- **Location**:
  - `crates/macos-agent/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/tests/preflight.rs`
- **Description**: Add end-to-end CLI tests that verify exit codes, stdout-only success contract,
  stderr-only error contract, and machine-readable schema shape for all key command families.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover positive and negative paths for preflight, activate, click, type, and hotkey.
  - Success-path tests assert stdout is non-empty JSON or TSV output and stderr is empty.
  - Error-path tests assert stdout is empty and stderr begins with `error:`.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent -- --nocapture`

### Task 4.3: Add shell completions and finalize user docs
- **Location**:
  - `completions/zsh/_macos-agent`
  - `completions/bash/macos-agent`
  - `crates/macos-agent/README.md`
- **Description**: Add Zsh and Bash completions for the final command set and update docs with
  permission matrix, failure modes, and practical recipes for agent usage.
- **Dependencies**:
  - Task 3.5
- **Complexity**: 4
- **Acceptance criteria**:
  - Completion files expose subcommands and key flags with valid value hints.
  - README includes a concise permissions troubleshooting table.
  - README documents fragility boundaries and recommended wait or retry practices.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `rg -n "Accessibility|Automation|Screen Recording|cliclick|retry" crates/macos-agent/README.md`

### Task 4.4: Run full workspace checks and coverage gate
- **Location**:
  - `DEVELOPMENT.md`
  - `crates/macos-agent/Cargo.toml`
- **Description**: Execute required lint, test, completion, and coverage commands from workspace
  policy. Ensure `macos-agent` is fully included in the same quality gate as existing CLIs.
- **Dependencies**:
  - Task 4.2
  - Task 4.3
- **Complexity**: 5
- **Acceptance criteria**:
  - All required commands in `DEVELOPMENT.md` pass.
  - Workspace coverage remains at or above 80.00 percent.
  - No command contract regressions in existing crates.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Testing Strategy
- Unit:
  - Command parsing, selector validation, retry policy, timeout handling, and output schema serialization.
- Integration:
  - End-to-end command execution with stubbed `osascript` and `cliclick` in deterministic test mode.
- E2E/manual:
  - macOS manual smoke for permission prompts, real app activation, click/type/hotkey actions, and
    screenshot observation.

## Risks & gotchas
- TCC permissions can change between terminal apps and can invalidate previously working automation.
- UI scripting reliability depends on animation timing, app responsiveness, and locale-dependent app names.
- External binary dependency on `cliclick` can fail at runtime due to missing install or PATH mismatch.
- `screen-record` reuse can introduce coupling risk if internal module structure changes.
- Keyboard automation can trigger in the wrong app if focus confirmation is skipped.

## Rollback plan
- Keep `macos-agent` isolated as a new binary with no behavior changes to existing CLIs.
- If rollout fails, remove `macos-agent` from workspace members and exclude new completions from release artifacts.
- Preserve `screen-record` public behavior by confining reuse changes to additive exports only.
- Revert to read-only observation commands (`preflight`, `windows list`, `observe screenshot`) while disabling mutating input commands behind a runtime feature flag if needed.
