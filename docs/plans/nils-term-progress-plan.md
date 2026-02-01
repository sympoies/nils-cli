# Plan: Add `nils-term` progress utilities crate

## Overview
This plan adds a new workspace crate, `nils-term`, that provides a small, RAII-friendly progress abstraction for other workspace crates/binaries to reuse. This iteration does not aim for 1:1 parity with the existing Zsh progress bar script; instead it wraps the `indicatif` crate to provide a determinate progress bar and an indeterminate spinner with consistent defaults. By default, progress output is sent to stderr and is automatically disabled when stderr is not a TTY, keeping stdout clean for command output and piping.

## Scope
- In scope:
  - Create a new library crate: `crates/nils-term`.
  - Provide minimal progress APIs (determinate + spinner) that are safe to use via RAII and are no-ops when disabled.
  - Use `indicatif` for rendering; prefer stderr as the draw target.
  - Add deterministic unit tests (capture output to a writer).
  - Add basic docs and one small workspace integration example to prevent bit-rot.
- Out of scope:
  - Exact behavioral parity with `/Users/terry/.config/zsh/scripts/progress-bar.zsh` (bar glyphs, update throttling rules, width heuristics, etc.).
  - A crates.io release or cross-repo shared crate.
  - Multi-progress UIs, nested progress trees, or rich TUI components.

## Assumptions (if any)
1. Rust stable is used and supports `std::io::IsTerminal` (already used elsewhere in this workspace).
2. Progress output should not interfere with stdout output (use stderr by default).
3. Workspace binaries should be able to opt out entirely (explicit disable) and rely on a safe default (auto-disable when not a TTY).

## Delivery gates (repo-wide)
- `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- `cargo test -p nils-term --doc`

## Adoption inventory (indicatif candidates in this workspace)
- High ROI (multi-item workloads):
  - `image-processing`:
    - `crates/image-processing/src/processing.rs` (`process_items`): loops over planned inputs and runs external tools per file (good determinate progress bar: `N / total`, message can include input/output basename).
    - `crates/image-processing/src/processing.rs` (`expand_inputs`): recursive directory scan can be slow on large trees (spinner while resolving inputs; optional).
  - `api-test` / `api-testing-core`:
    - `crates/api-testing-core/src/suite/runner.rs` (`run_suite`): runs suites case-by-case (`loaded.manifest.cases`), with optional fail-fast (good determinate bar per case; message can include case id + status).
    - `crates/api-test/src/main.rs` prints results JSON to stdout, so stderr-based progress is safe and won’t corrupt machine-readable output.
- Medium ROI (pre-computation before interactive UX):
  - `fzf-cli`:
    - `crates/fzf-cli/src/file.rs` (`list_files`) and `crates/fzf-cli/src/directory.rs` (`list_dirs`, `list_files_in_dir`) traverse via `WalkDir` + sort (spinner while indexing before launching `fzf`; ensure it finishes before entering `fzf`).
    - `crates/fzf-cli/src/defs/index.rs` (`build_index`): scans/reads/parses many `.zsh` files to build an index (spinner or simple determinate if file count is known).
- Low/conditional ROI (script-friendly CLIs):
  - `plan-tooling`:
    - `crates/plan-tooling/src/validate.rs` validates multiple `docs/plans/*-plan.md` files (progress would need to be opt-in to avoid noisy stderr in CI/automation).
- Parity-sensitive (avoid unless explicitly opt-in):
  - `git-scope`:
    - Print modes (`-p`) can process/print many files, but `git-scope` prioritizes behavioral parity with the original script; adding progress output should be strictly opt-in (flag/env) to avoid changing expected UX.

## Sprint 1: Crate scaffold + API contract
**Goal**: `nils-term` exists and exposes a small, stable public API that is easy to adopt across workspace binaries.
**Demo/Validation**:
- Command(s):
  - `cargo check -p nils-term`
  - `cargo test -p nils-term`
- Verify:
  - The crate builds on its own.
  - Public API docs compile and describe default behavior (stderr + auto TTY enablement).

### Task 1.1: Scaffold `nils-term` and wire it into the workspace
- **Location**:
  - `Cargo.toml`
  - `crates/nils-term/Cargo.toml`
  - `crates/nils-term/src/lib.rs`
- **Description**: Add `crates/nils-term` as a new workspace member. Add `indicatif` (and any minimal supporting deps, if needed) to `[workspace.dependencies]` and depend on it from `nils-term`. Keep the initial crate layout small: `lib.rs` + a `progress` module.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - `cargo check -p nils-term` passes.
  - The crate has a clear module structure (e.g. `nils_term::progress`).
  - No other workspace crates are modified yet beyond workspace wiring.
- **Validation**:
  - `cargo check -p nils-term`
  - `rg -n "crates/nils-term" Cargo.toml`

### Task 1.2: Define the public API and defaults (RAII, stderr, auto-enable)
- **Location**:
  - `crates/nils-term/src/lib.rs`
  - `crates/nils-term/src/progress.rs`
- **Description**: Define a minimal, non-leaky API that does not require downstream crates to depend on `indicatif` types. Prefer a small options struct + RAII types. Make the contract explicit by documenting the public exports and behavior:
  - Public exports:
    - `ProgressEnabled` (`Auto | On | Off`).
    - `ProgressOptions` (at minimum: `enabled`, `prefix`, optional fixed `width` for tests).
    - `DeterminateProgress` and `SpinnerProgress`.
  - Required methods (exact names can vary, but must cover these behaviors):
    - Create: `DeterminateProgress::new(total, options)` and `SpinnerProgress::new(options)`.
    - Update: `set_position/inc` for determinate; `tick` for spinner; optional `set_message`/`set_suffix`.
    - Finish: explicit `finish(...)` that terminates cleanly (and is idempotent).
  - `ProgressEnabled::Auto` rules:
    - When drawing to stderr (default), auto-enable only when `stderr.is_terminal()`.
    - In tests (writer-based draw target), auto-enable must not be blocked by TTY detection (tests must be able to force enabled behavior deterministically).
  - `Drop` behavior:
    - Must not panic.
    - Must not spawn background ticking by default.
    - Must not emit “finish” output if nothing was rendered (avoid surprising blank lines).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - API users can create determinate/spinner progress without importing `indicatif`.
  - Defaults are documented: draw to stderr; auto-disable when stderr is not a TTY.
  - Disabled progress is a no-op and does not allocate background threads/timers.
- **Validation**:
  - `cargo check -p nils-term`
  - `cargo test -p nils-term`

### Task 1.3: Implement an `indicatif`-backed renderer with stable templates
- **Location**:
  - `crates/nils-term/src/progress.rs`
- **Description**: Implement the progress types using `indicatif` with a consistent style:
  - Determinate: show prefix + bar + counters; suffix/message is optional.
  - Spinner: show prefix + spinner; suffix/message is optional.
  - Provide a deterministic mode for tests by allowing a fixed width and a `ProgressDrawTarget::to_writer(...)` draw target.
  - Avoid time-based draw throttling in test mode (tests should not sleep or depend on timers).
  - Ensure a safe fallback when a style/template cannot be constructed (do not panic).
  - Ensure finishing behavior is consistent: explicit `finish(...)` ends the bar/spinner and ensures the terminal line is not left mid-update; `Drop` should call a safe finish path if needed.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Determinate progress updates and can finish cleanly.
  - Spinner can tick and finish cleanly.
  - Auto mode disables rendering when stderr is not a TTY.
- **Validation**:
  - `cargo test -p nils-term`

## Sprint 2: Deterministic tests + docs + adoption example
**Goal**: `nils-term` is test-covered and has at least one in-workspace consumer/example to keep it healthy.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-term`
  - `cargo check -p cli-template`
- Verify:
  - Tests are deterministic and do not rely on timing or real terminal state.
  - A small demo command compiles and runs (progress to stderr, output to stdout remains stable).

### Task 2.1: Add deterministic output-capture tests for progress rendering
- **Location**:
  - `crates/nils-term/tests/progress.rs`
- **Description**: Add tests that render progress into an in-memory writer using `indicatif`’s draw target APIs. Cover:
  - Disabled mode produces no output.
  - Determinate mode renders at least one update and finishes (newline/termination behavior is consistent).
  - Spinner mode renders at least one tick and finishes.
  - Fixed width is honored in test mode (avoid terminal-width dependence).
  - Output assertions are resilient:
    - Prefer asserting on key substrings rather than full-line snapshots.
    - Normalize or ignore carriage returns (`\r`) if needed to avoid brittle comparisons.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - `cargo test -p nils-term` passes reliably on CI and locally.
  - Tests do not sleep or rely on wall-clock timing.
- **Validation**:
  - `cargo test -p nils-term`

### Task 2.2: Add rustdoc examples and “how to use” docs
- **Location**:
  - `crates/nils-term/src/lib.rs`
- **Description**: Add concise docs showing:
  - A determinate progress use-case (e.g. iterating over N items).
  - A spinner use-case for “loading” work.
  - Default behavior: stderr + auto-disable when not a TTY.
  - Guidance for libraries: accept a `ProgressEnabled`/`ProgressOptions` from callers rather than reading env vars internally.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Examples compile as doc tests.
  - Public API docs explain defaults and how to disable progress.
- **Validation**:
  - `cargo test -p nils-term --doc`

### Task 2.3: Add a small integration demo in `cli-template`
- **Location**:
  - `crates/cli-template/Cargo.toml`
  - `crates/cli-template/src/main.rs`
- **Description**: Add a small subcommand (e.g. `progress-demo`) that uses `nils-term` to render a short progress sequence to stderr while keeping stdout output stable. The goal is only to ensure the crate is exercised by at least one binary in this repo.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo check -p cli-template` passes.
  - Running the demo shows progress when stderr is a TTY and runs silently when stderr is not a TTY.
  - Demo progress output is not interleaved with noisy logging while progress is active (keep the demo readable).
- **Validation**:
  - `cargo check -p cli-template`
  - `cargo run -q -p cli-template -- --log-level error progress-demo`

## Testing Strategy
- Unit:
  - `nils-term` output-capture tests using `ProgressDrawTarget::to_writer(...)`.
  - Disabled-mode tests that assert no output is produced.
- Integration:
  - `cli-template` demo subcommand build + run as a lightweight smoke test.
- E2E/manual:
  - Run a real CLI that uses `nils-term` and confirm stdout piping remains clean (progress appears only on stderr).

## Risks & gotchas
- `indicatif` output can be sensitive to draw targets and terminal-width detection. Mitigation: use fixed width + `to_writer` in tests.
- RAII + `Drop` can cause surprising output if a progress object is dropped without rendering. Mitigation: track “rendered at least once” and only emit finish output when needed.
- Some CI environments set pseudo-TTYs; relying on `is_terminal()` is usually fine but tests should not depend on it. Mitigation: tests should force draw target and “enabled” setting explicitly.
- Progress rendering on stderr can be disrupted by other stderr output (e.g. logging). Mitigation: keep progress usage scoped; avoid emitting logs while progress is actively updating (or route logs elsewhere for specific commands).
- `NO_COLOR` / `TERM=dumb` environments may expect simplified output. Mitigation: document the behavior and rely on `indicatif`/terminal defaults rather than adding bespoke env-var parsing in v1.

## Rollback plan
- Remove `crates/nils-term` from workspace members and delete the crate directory.
- Remove `indicatif` from workspace dependencies.
- Revert the `cli-template` demo integration (and any references/imports).
- Revert any `Cargo.lock` changes introduced by adding dependencies.
