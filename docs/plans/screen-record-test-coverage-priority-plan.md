# Plan: Prioritize uncovered test coverage for `screen-record`

## Overview
This plan raises `crates/screen-record` coverage by targeting the highest-risk uncovered behavior first: CLI mode/flag orchestration in `src/run.rs`, then Linux runtime error paths (`preflight`/`ffmpeg`/portal/X11), then residual helper gaps. Current crate baseline from `target/coverage/screen-record.lcov.info` is **74.87%** (882/1178), with `src/run.rs` at **64.26%** (453/705) and **252 missed lines**. The strategy is to add deterministic unit and integration tests (fixture/stub-first), avoid behavior changes, and keep validation runnable on both macOS (local) and Linux (CI).

## Scope
- In scope:
  - Add high-value tests for uncovered branches in `run.rs` (mode gating, selector validation, portal-specific paths, path/format resolution, filename sanitization).
  - Add Linux-only tests for `preflight`, `ffmpeg`/audio error handling, portal response parsing, and X11 fallback/ordering behavior.
  - Add low-cost residual tests for helper modules (`error`, `test_mode`, `select`) after major gaps are covered.
  - Use crate-scoped coverage checks to guide and verify progress.
- Out of scope:
  - Changing CLI behavior, output contracts, or error text semantics.
  - Rewriting platform backends (ScreenCaptureKit, DBus portal, X11) beyond testability seams.
  - Raising workspace-wide CI coverage gates in this effort.

## Assumptions (if any)
1. Coverage is measured with:
   - `cargo llvm-cov nextest --profile ci -p screen-record --lcov --output-path target/coverage/screen-record.lcov.info`
   - `scripts/ci/coverage-summary.sh target/coverage/screen-record.lcov.info`
2. Linux-specific tests run in CI (Ubuntu 24.04) and may be skipped locally on macOS due `cfg(target_os = "linux")`.
3. Test additions must be hermetic:
   - no external network calls,
   - no dependence on globally installed GUI/audio tools unless explicitly stubbed,
   - no dependence on user machine state.

## Acceptance criteria
- `screen-record` crate line coverage reaches **>= 82.00%** on the crate coverage command.
- `crates/screen-record/src/run.rs` line coverage reaches **>= 78.00%**.
- New tests cover currently under-tested critical behavior:
  - portal flag/mode gating and non-Linux rejection paths,
  - screenshot/record output path + format conflict rules,
  - Linux preflight/portal/ffmpeg actionable failure paths.
- Required repository checks pass before delivery:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Validation
- `cargo llvm-cov nextest --profile ci -p screen-record --lcov --output-path target/coverage/screen-record.lcov.info`
- `scripts/ci/coverage-summary.sh target/coverage/screen-record.lcov.info`
- `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Sprint 1: Cover highest-impact CLI orchestration gaps (`run.rs` first)
**Goal**: Close major uncovered branches in the core CLI dispatcher and argument validation logic before secondary modules.
**Demo/Validation**:
- Command(s):
  - `cargo test -p screen-record --test recording_test_mode --test selection --test cli_smoke`
  - `cargo test -p screen-record run::tests`
  - `cargo llvm-cov nextest --profile ci -p screen-record --lcov --output-path target/coverage/screen-record.lcov.info`
- Verify:
  - `run.rs` coverage rises materially from the 64.26% baseline.
  - High-risk mode/flag conflicts and portal behavior have deterministic tests.

**Parallel lanes**:
- Lane A: Task 1.1
- Lane B: Task 1.2
- Lane C: Task 1.3

### Task 1.1: Build a reproducible hotspot inventory for `screen-record`
- **Location**:
  - `target/coverage/screen-record.lcov.info`
  - `notes/screen-record-coverage-hotspots.md`
- **Description**: Add a small, reproducible hotspot note for this crate that records baseline totals, top uncovered files/functions, and target checkpoints used by this plan (so test priority does not drift by intuition).
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Baseline totals and top uncovered areas are documented with exact numbers.
  - The note identifies Sprint 1 target branches in `run.rs` and expected coverage delta.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci -p screen-record --lcov --output-path target/coverage/screen-record.lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/screen-record.lcov.info`
  - `test -s notes/screen-record-coverage-hotspots.md`

### Task 1.2: Expand mode/flag validation matrix tests in `run.rs`
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/tests/recording_test_mode.rs`
- **Description**: Add deterministic tests for currently weakly covered validation and dispatch branches:
  - `--portal` invalid mode usage and non-Linux rejection messaging.
  - `--window-name` requires `--app` for both record/screenshot modes.
  - screenshot-mode invalid recording flags (`--format`, etc.).
  - preflight/request modes rejecting capture/recording flags via `ensure_no_recording_flags`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Each validation branch has at least one passing regression test.
  - Error codes remain `2` for usage errors and messages stay actionable.
  - No changes to CLI behavior/output contract.
- **Validation**:
  - `cargo test -p screen-record --test recording_test_mode`
  - `cargo test -p screen-record run::tests`

### Task 1.3: Add tests for output path/format resolution and portal screenshot naming
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/tests/recording_test_mode.rs`
- **Description**: Add focused tests for unresolved helper behavior in output resolution:
  - `resolve_output_path` relative-path canonicalization + parent dir creation.
  - `resolve_container` conflict handling and extension fallback defaults.
  - `resolve_image_format` valid/invalid extension + explicit format conflict combinations.
  - portal screenshot auto-path generation (`screenshot-<ts>-portal`), collision suffix (`-2`), and `--dir` file-vs-directory validation.
  - filename segment sanitization edge cases (punctuation, control chars, truncation).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Tests cover both success and error branches with stable assertions.
  - Path generation remains deterministic in test mode (`CODEX_SCREEN_RECORD_TEST_TIMESTAMP`).
  - `run.rs` uncovered helper lines are reduced in the next coverage checkpoint.
- **Validation**:
  - `cargo test -p screen-record --test recording_test_mode`
  - `cargo test -p screen-record run::tests`

## Sprint 2: Linux runtime/error-path coverage hardening
**Goal**: Cover Linux-only branches that are critical for actionable failures and robust diagnostics in real environments.
**Demo/Validation**:
- Command(s):
  - `cargo test -p screen-record --tests`
  - `cargo test -p screen-record --test linux_unit --test linux_portal_unit --test linux_request_permission --test linux_x11_integration`
  - `cargo llvm-cov nextest --profile ci -p screen-record --lcov --output-path target/coverage/screen-record.lcov.info`
- Verify:
  - Linux failures produce deterministic, actionable error messages.
  - Portal/X11/ffmpeg fallback and error branches are tested without external dependencies.

**Parallel lanes**:
- Lane A: Task 2.1
- Lane B: Task 2.2
- Lane C: Task 2.3

### Task 2.1: Add unit coverage for Linux preflight parsing and session checks
- **Location**:
  - `crates/screen-record/src/linux/preflight.rs`
  - `crates/screen-record/tests/linux_request_permission.rs`
- **Description**: Add tests for `x11_socket_path` parsing edge cases and preflight branching:
  - accepted forms (`:0`, `unix:1.0`, `localhost:2`),
  - rejected remote host forms,
  - Wayland-only branch with portal available/missing,
  - no-display runtime error branch.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Socket parsing behavior is deterministic and documented by tests.
  - Wayland/no-display branches are both covered and produce stable guidance.
- **Validation**:
  - `cargo test -p screen-record --test linux_request_permission`

### Task 2.2: Expand ffmpeg/audio error-path tests with stubs
- **Location**:
  - `crates/screen-record/src/linux/ffmpeg.rs`
  - `crates/screen-record/src/linux/audio.rs`
  - `crates/screen-record/tests/linux_unit.rs`
- **Description**: Extend Linux unit tests beyond happy paths:
  - `pactl` missing / spawn failures / empty outputs.
  - monitor source missing (`<sink>.monitor` not found).
  - `ffmpeg -devices`/`-h demuxer=pipewire` failure diagnostics.
  - `run_ffmpeg` non-zero exit path with stderr snippet and exit code suffix.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 9
- **Acceptance criteria**:
  - At least one test exists for each targeted failure branch.
  - Error messages remain actionable and match current contract text.
  - Tests remain hermetic via `StubBinDir` and env guards.
- **Validation**:
  - `cargo test -p screen-record --test linux_unit`

### Task 2.3: Cover portal response parsing and dictionary conversion failures
- **Location**:
  - `crates/screen-record/src/linux/portal.rs`
  - `crates/screen-record/tests/linux_portal_unit.rs`
- **Description**: Add unit tests for portal helper logic that currently lacks branch coverage:
  - `env_flag_enabled` normalization (`1/true/yes/on` and falsey variants),
  - `dict_get_objpath` / `dict_get_streams` missing key and type-mismatch paths,
  - non-zero portal response code handling as runtime error.
  - If needed, extract tiny pure helpers from DBus-bound code to test parsing without live DBus.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Parsing/conversion error paths are covered without requiring a live portal service.
  - Existing test-mode bypass behavior (`TEST_PIPEWIRE_NODE_ID`) stays unchanged.
- **Validation**:
  - `cargo test -p screen-record --test linux_portal_unit`
  - `cargo test -p screen-record --tests`

## Sprint 3: Residual gaps, stabilization, and final gate
**Goal**: Close low-cost remaining misses, then verify coverage and required repository checks.
**Demo/Validation**:
- Command(s):
  - `cargo test -p screen-record`
  - `cargo llvm-cov nextest --profile ci -p screen-record --lcov --output-path target/coverage/screen-record.lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/screen-record.lcov.info`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - Coverage targets are met.
  - No regressions in required workspace checks.

**Parallel lanes**:
- Lane A: Task 3.1
- Lane B: Task 3.2
- Lane C: Task 3.3 (after Task 3.2)

### Task 3.1: Close low-effort helper branch gaps
- **Location**:
  - `crates/screen-record/tests/error.rs`
  - `crates/screen-record/tests/writer.rs`
  - `crates/screen-record/tests/selection.rs`
  - `crates/screen-record/src/run.rs`
- **Description**: Add targeted tests for small uncovered branches:
  - `CliError::unsupported_platform` message + exit code.
  - `TestWriter::finish()` error when no frame was appended.
  - `select_window` no-match + z-order tie paths not currently covered.
  - `normalize_tsv_field` newline/tab normalization for app/display output helpers.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Small helper gaps are covered with concise deterministic tests.
  - No behavior drift in public CLI contract.
- **Validation**:
  - `cargo test -p screen-record --test error --test writer --test selection`

### Task 3.2: Coverage-driven last-mile pass on top missed `run.rs` lines
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/tests/recording_test_mode.rs`
- **Description**: Re-run crate coverage, inspect remaining top misses in `run.rs`, and add only high-value tests needed to hit plan thresholds (avoid redundant/low-signal tests).
- **Dependencies**:
  - Task 1.2
  - Task 1.3
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Crate coverage >= 82.00%.
  - `run.rs` coverage >= 78.00%.
  - Remaining misses are documented and intentionally deferred (if any).
- **Validation**:
  - `cargo llvm-cov nextest --profile ci -p screen-record --lcov --output-path target/coverage/screen-record.lcov.info`
  - `scripts/ci/coverage-summary.sh target/coverage/screen-record.lcov.info`

### Task 3.3: Run required repo checks and stabilize
- **Location**:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- **Description**: Execute mandatory repo checks, fix test/lint regressions caused by new tests, and ensure cross-crate checks still pass.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Required checks pass with no skipped failing step.
  - No newly introduced flaky tests remain unresolved in this scope.
- **Validation**:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit:
  - `run.rs` validation/path/format helper tests.
  - Linux pure helper tests in `preflight.rs`, `audio.rs`, `portal.rs`.
- Integration:
  - `recording_test_mode.rs` for deterministic CLI contract checks.
  - Linux integration tests for X11 listing/selection + ffmpeg argument routing.
- E2E/manual:
  - Optional manual smoke on Linux X11 and Wayland portal environments after CI passes.

## Risks & gotchas
- Platform asymmetry:
  - Linux-only branches are not executable on macOS local runs.
  - Mitigation: keep Linux coverage tasks validated in Ubuntu CI and avoid introducing OS-coupled flakiness.
- Global state in tests (`PATH`, `DISPLAY`, `WAYLAND_DISPLAY`, `CODEX_*` env vars):
  - Mitigation: consistently use `GlobalStateLock` + `EnvGuard` and scoped temp dirs.
- Overfitting tests to implementation details:
  - Mitigation: assert stable contract text/exit codes and generated path shapes, not incidental internals.

## Rollback plan
- If new tests introduce instability:
  - Revert only the new test files/sections first (`crates/screen-record/tests/*` and `run.rs` test module additions), keeping production code unchanged.
  - Keep a minimal stable subset: mode/flag validation tests and one test per major error path.
  - Re-run `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh` to confirm baseline stability before re-introducing tests incrementally.
