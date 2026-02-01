# Plan: Add `nils-term` progress utilities crate

## Overview
This plan adds a new workspace crate, `nils-term`, that provides a small, RAII-friendly progress abstraction for other workspace crates (libraries and binaries) to reuse. This iteration does not aim for 1:1 parity with the existing Zsh progress bar script; instead it wraps the `indicatif` crate and standardizes enablement + draw target + finish behavior across multiple CLIs. By default, progress output is sent to stderr and is automatically disabled when stderr is not a TTY, keeping stdout clean for command output and piping (especially important for JSON-producing CLIs like `api-test`).

## Scope
- In scope:
  - Create a new library crate: `crates/nils-term`.
  - Provide minimal progress APIs (determinate + spinner) that are safe to use via RAII, `Clone` (passable into deeper library code), and are no-ops when disabled.
  - Use `indicatif` for rendering; prefer stderr as the draw target; ensure finish behavior supports both “leave” and “clear” use cases.
  - Add deterministic unit tests (capture output to a writer).
  - Add basic docs and adopt into representative CLIs to prevent bit-rot:
    - `api-test` / `api-testing-core` (library + JSON-safe stdout)
    - `image-processing` (batch file processing)
    - `fzf-cli` (interactive pre-indexing spinner)
    - `api-rest` / `api-gql` (JSON-safe stdout; spinner for network I/O + report generation)
    - `git-summary` (multi-author batch processing)
    - `git-lock` (lock dir scanning + per-entry enrichment)
    - `plan-tooling` (interactive UX only; must not spam automation)
  - Out of scope:
  - Exact behavioral parity with `https://github.com/graysurf/zsh-kit/blob/main/scripts/progress-bar.zsh` (bar glyphs, update throttling rules, width heuristics, etc.).
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
  - `git-summary`:
    - `crates/git-summary/src/main.rs` (`render_summary`): enumerates authors then runs per-author `git log --numstat` (good determinate progress over authors).
- Medium ROI (pre-computation before interactive UX):
  - `fzf-cli`:
    - `crates/fzf-cli/src/file.rs` (`list_files`) and `crates/fzf-cli/src/directory.rs` (`list_dirs`, `list_files_in_dir`) traverse via `WalkDir` + sort (spinner while indexing before launching `fzf`; ensure it finishes before entering `fzf`).
    - `crates/fzf-cli/src/defs/index.rs` (`build_index`): scans/reads/parses many `.zsh` files to build an index (spinner or simple determinate if file count is known).
  - `api-rest` / `api-gql`:
    - `crates/api-rest/src/main.rs` (`cmd_call_internal`): waits on network I/O for request execution (spinner around `execute_rest_request` and cleanup; stdout must remain untouched).
    - `crates/api-gql/src/main.rs` (`cmd_call`): waits on network I/O for operation execution (spinner around request; stdout prints JSON).
    - `crates/api-rest/src/main.rs` / `crates/api-gql/src/main.rs` (`cmd_report`): optional `--run` path performs a request then generates a report (spinner for the request; determinate progress if iterating many history entries).
  - `git-lock`:
    - `crates/git-lock/src/list.rs` (`collect_entries`): scans and parses lock files (spinner or determinate if count is known; finish before printing the list).
- Low/conditional ROI (script-friendly CLIs):
  - `plan-tooling`:
    - `crates/plan-tooling/src/validate.rs` validates multiple `docs/plans/*-plan.md` files (progress would need to be opt-in to avoid noisy stderr in CI/automation).
  - `semantic-commit`:
    - `crates/semantic-commit/src/commit.rs` runs `git commit` with inherited stderr (progress should not render concurrently; only consider progress for pre-flight steps if it stays readable).
- Parity-sensitive (avoid unless explicitly opt-in):
  - `git-scope`:
    - Print modes (`-p`) can process/print many files, but `git-scope` prioritizes behavioral parity with the original script; adding progress output should be strictly opt-in (flag/env) to avoid changing expected UX.

## Cross-CLI requirements (derived from the inventory)
- Enablement and output:
  - Must be safe for machine-readable stdout (progress should default to stderr).
  - Must be easy to disable per CLI (script-friendly commands should be able to default `Off` even when run in a TTY).
  - `Auto` enablement should be based on the chosen draw target’s `is_terminal()` (no hidden global state).
- API ergonomics:
  - Must be usable from libraries (e.g. `api-testing-core`) with enablement decided by the binary; avoid env var parsing inside library code.
  - Must be cheap/no-op when disabled so callers can pass a progress handle unconditionally.
- Rendering semantics:
  - Must support both determinate (known total) and spinner (unknown total) workflows.
  - Must support message updates (per-item/per-case labels) without requiring callers to depend on `indicatif` types.
  - Must support clear/leave finish behaviors (e.g. clear before launching `fzf`; leave a final line for batch runs).
  - Must avoid background ticking by default (callers opt into manual `tick()` / updates).
  - Must allow “print safely while progress exists” (either by forcing clear-before-print or via a `suspend` helper).

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
- **Description**: Define a minimal, non-leaky API that does not require downstream crates to depend on `indicatif` types. The API must be usable from both binaries and libraries (passable handles; no-op when disabled). Make the contract explicit by documenting the public exports and behavior:
  - Public exports:
    - `ProgressEnabled` (`Auto | On | Off`).
    - `ProgressFinish` (`Leave | Clear`) to support both batch and interactive (pre-`fzf`) use.
    - `ProgressOptions` (at minimum: `enabled`, `prefix`, optional fixed `width` for tests, and `finish` behavior).
    - `Progress` (a single clonable handle type that can be constructed as determinate or spinner).
  - Required methods (exact names can vary, but must cover these behaviors):
    - Create: `Progress::new(total, options)` and `Progress::spinner(options)`.
    - Update: `set_position/inc`, `tick`, and `set_message` (message should work for both modes).
    - Finish: explicit `finish()` / `finish_with_message(...)` and `finish_and_clear()` (or a single `finish(mode)`), all idempotent.
    - Output coordination: `suspend(|| ...)` helper so callers can safely print to stderr without leaving the terminal mid-progress.
  - `ProgressEnabled::Auto` rules:
    - When drawing to stderr (default), auto-enable only when `stderr.is_terminal()`.
    - In tests (writer-based draw target), auto-enable must not be blocked by TTY detection (tests must be able to force enabled behavior deterministically).
  - `Drop` behavior:
    - Must not panic.
    - Must not spawn background ticking by default.
    - Must not emit “finish” output if nothing was rendered (avoid surprising blank lines).
    - Must respect the configured finish behavior (leave vs clear) when a progress was rendered.
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
- **Description**: Implement the progress type using `indicatif` with a consistent style:
  - Determinate: show prefix + bar + counters; message is optional.
  - Spinner: show prefix + spinner; message is optional.
  - Provide a deterministic mode for tests by allowing a fixed width and a `ProgressDrawTarget::to_writer(...)` draw target.
  - Avoid time-based draw throttling in test mode (tests should not sleep or depend on timers).
  - Ensure a safe fallback when a style/template cannot be constructed (do not panic).
  - Ensure finishing behavior supports both leave and clear modes and does not leave the terminal mid-update.
  - Implement `suspend(|| ...)` so progress rendering can be temporarily paused while emitting normal stderr output.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Determinate progress updates and can finish cleanly.
  - Spinner can tick and finish cleanly.
  - Auto mode disables rendering when stderr is not a TTY.
- **Validation**:
  - `cargo test -p nils-term`

## Sprint 2: Deterministic tests + docs + minimal consumer example
**Goal**: `nils-term` is test-covered, documented, and has at least one small consumer example to keep it healthy.
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
  - `suspend(|| ...)` does not panic and does not corrupt the captured output stream.
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
  - Guidance for libraries: accept a `Progress` (or `ProgressOptions`) from callers rather than reading env vars internally.
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

## Sprint 3: Prove cross-CLI fit (library + batch + interactive)
**Goal**: Validate the `nils-term` API fits the real-world patterns in this repo (library code, batch file processing, and interactive pre-work).
**Demo/Validation**:
- Command(s):
  - `cargo test -p api-testing-core`
  - `cargo test -p api-test`
  - `cargo test -p image-processing`
  - `cargo test -p fzf-cli`
- Verify:
  - `api-test` keeps stdout JSON clean while showing progress on stderr in TTY.
  - `image-processing` can show determinate progress for multi-file operations without affecting stdout output.
  - `fzf-cli` can show a spinner during indexing and clears it before invoking `fzf`.

### Task 3.1: Add optional progress plumbing to `api-testing-core` (library-safe)
- **Location**:
  - `crates/api-testing-core/src/suite/runner.rs`
  - `crates/api-testing-core/src/suite/runner.rs` (options structs)
- **Description**: Thread an optional `nils_term::progress::Progress` handle through `run_suite` so the binary can decide enablement. Update the case loop to update position/message per executed case, and finish at the end. Ensure disabled/no-op progress imposes minimal overhead.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - `api-testing-core` does not read env vars to decide progress; it only uses the passed-in handle/options.
  - Progress updates do not change stdout behavior of callers.
  - Unit/integration tests still pass.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 3.2: Enable progress in `api-test run` without breaking JSON stdout
- **Location**:
  - `crates/api-test/src/main.rs`
- **Description**: Construct a `nils-term` determinate progress for the suite run and pass it into `api-testing-core`. Keep progress output on stderr and ensure it is automatically disabled when stderr is not a TTY. Avoid extra stderr logging while progress is actively rendering.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `api-test` still prints a single JSON line to stdout (unchanged).
  - In a TTY, progress is visible on stderr; when stderr is not a TTY, it is silent.
- **Validation**:
  - `cargo test -p api-test`

### Task 3.3: Add determinate progress to `image-processing` batch operations
- **Location**:
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/processing.rs`
- **Description**: Create a determinate progress for the planned input list and update it after each item is processed. Use leave-vs-clear behavior appropriate for batch output (prefer leaving a final line). Ensure progress does not affect JSON output mode.
- **Dependencies**:
  - Task 1.3
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Multi-file operations show progress in TTY without changing stdout content.
  - JSON output remains valid and unchanged (stdout-only).
- **Validation**:
  - `cargo test -p image-processing`

### Task 3.4: Add spinner progress to `fzf-cli` indexing paths (clear before `fzf`)
- **Location**:
  - `crates/fzf-cli/src/file.rs`
  - `crates/fzf-cli/src/directory.rs`
  - `crates/fzf-cli/src/defs/index.rs`
- **Description**: Add a spinner for the potentially slow “build input list” phases (WalkDir scans, indexing) and clear it before launching `fzf` or printing output. Ensure the spinner is no-op when not in a TTY.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Spinner appears during indexing in TTY and does not remain on screen after `fzf` starts.
  - `fzf-cli` stdout output remains unchanged.
- **Validation**:
  - `cargo test -p fzf-cli`

## Sprint 4: Adopt into remaining workspace CLIs
**Goal**: Apply `nils-term` to the remaining candidate binaries in this workspace so progress behavior is consistent and reusable.
**Demo/Validation**:
- Command(s):
  - `cargo test -p api-rest`
  - `cargo test -p api-gql`
  - `cargo test -p git-summary`
  - `cargo test -p git-lock`
  - `cargo test -p plan-tooling`
- Verify:
  - All progress output goes to stderr only.
  - In non-TTY environments, progress is silent (Auto mode).
  - Progress is cleared before printing error blocks to stderr (readable failures).

### Task 4.1: Add spinner progress to `api-rest` call/report execution paths
- **Location**:
  - `crates/api-rest/src/main.rs`
- **Description**: Add a spinner around the network-bound execution path in `cmd_call_internal` (wrap `api_testing_core::rest::runner::execute_rest_request` and cleanup). Ensure any errors are printed after progress is finished/cleared (use `finish_and_clear` or `suspend`). For `cmd_report --run`, show a spinner while the embedded call runs.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 5
- **Acceptance criteria**:
  - `api-rest` stdout output is unchanged (response body stays on stdout).
  - In a TTY, users see a spinner while the request runs; outside TTY, no progress is emitted.
  - Error output remains readable and is not interleaved with an active progress line.
- **Validation**:
  - `cargo test -p api-rest`

### Task 4.2: Add spinner progress to `api-gql` call/report execution paths
- **Location**:
  - `crates/api-gql/src/main.rs`
- **Description**: Add a spinner around the network-bound execution path in `cmd_call` and the `cmd_report --run` path. Ensure progress is finished/cleared before printing errors to stderr.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 5
- **Acceptance criteria**:
  - `api-gql` stdout output is unchanged (response JSON stays on stdout).
  - In a TTY, users see a spinner while the request runs; outside TTY, no progress is emitted.
  - Error output remains readable and is not interleaved with an active progress line.
- **Validation**:
  - `cargo test -p api-gql`

### Task 4.3: Add determinate progress to `git-summary` author aggregation
- **Location**:
  - `crates/git-summary/src/main.rs`
- **Description**: Add a determinate progress bar over the author loop in `render_summary`: after `collect_authors`, set total = number of authors, set message to current author, and update position after each `collect_author_row`. Finish/clear before printing the final table to stdout.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Stdout output (the final table) is unchanged and remains clean/pipeline-safe.
  - In a TTY, users see progress while per-author logs are computed; outside TTY, no progress is emitted.
- **Validation**:
  - `cargo test -p git-summary`

### Task 4.4: Add spinner/determinate progress to `git-lock list` scan phase
- **Location**:
  - `crates/git-lock/src/list.rs`
- **Description**: Add a progress indicator while scanning/parsing lock files in `collect_entries`. Finish/clear the progress before printing the list to stdout so output stays readable. (If file count can be known cheaply, use a determinate bar; otherwise use a spinner.)
- **Dependencies**:
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - `git-lock list` stdout output remains unchanged.
  - In a TTY, users see a progress indicator only during the scan phase; outside TTY, it is silent.
- **Validation**:
  - `cargo test -p git-lock`

### Task 4.5: Add optional progress to `plan-tooling validate`
- **Location**:
  - `crates/plan-tooling/src/validate.rs`
- **Description**: Add progress over the discovered plan files list so interactive runs show forward movement. Ensure all progress output is on stderr and is disabled in non-TTY environments by default. Before printing validation errors, finish/clear progress so error blocks are readable.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - `plan-tooling validate` behavior is unchanged in CI/non-TTY environments (no extra stderr noise).
  - In a TTY, progress shows per-file validation and is cleared before error output.
- **Validation**:
  - `cargo test -p plan-tooling`
  - `cargo run -q -p plan-tooling -- validate --file docs/plans/nils-term-progress-plan.md`

## Sprint 5: Parity-sensitive and awkward integrations (explicit opt-in)
**Goal**: Handle remaining binaries where progress output risks changing expected UX; keep defaults unchanged and make progress opt-in.
**Demo/Validation**:
- Command(s):
  - `cargo test -p semantic-commit`
  - `cargo test -p git-scope`
- Verify:
  - Default user-facing output is unchanged unless an explicit opt-in is provided.

### Task 5.1: Evaluate `semantic-commit` progress viability (likely limited to pre-flight)
- **Location**:
  - `crates/semantic-commit/src/commit.rs`
- **Description**: Evaluate whether progress output can be added without interleaving with inherited stderr from `git commit`. If viable, add progress only for pre-flight steps (git repo checks, staged-change detection, message validation) and ensure it is finished/cleared before invoking `git commit` (which may write to stderr). If not viable, document in code comments/docs why progress is intentionally not used.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 5
- **Acceptance criteria**:
  - No progress output is shown while `git commit` is actively emitting output to stderr.
  - Default behavior remains unchanged for non-TTY environments.
- **Validation**:
  - `cargo test -p semantic-commit`

### Task 5.2: Add opt-in progress to `git-scope` for print-heavy modes (maintain parity by default)
- **Location**:
  - `crates/git-scope/src/main.rs`
  - `crates/git-scope/src/print.rs`
  - `crates/git-scope/tests`
- **Description**: If (and only if) acceptable for parity, add an explicit opt-in mechanism (flag or env var) to enable progress in print-heavy paths (e.g. `-p` modes that print many files). Keep default behavior and stdout output identical when not opted in. Add regression coverage to ensure default-mode stdout is unchanged and progress goes to stderr only.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Without opt-in, existing output comparisons for `git-scope` remain unchanged.
  - With opt-in, progress is visible in TTY and is silent in non-TTY.
- **Validation**:
  - `cargo test -p git-scope`

## Testing Strategy
- Unit:
  - `nils-term` output-capture tests using `ProgressDrawTarget::to_writer(...)`.
  - Disabled-mode tests that assert no output is produced.
- Integration:
  - `cli-template` demo subcommand build + run as a lightweight smoke test.
  - `api-test` suite-run uses progress via `api-testing-core` without stdout changes.
  - `image-processing`, `fzf-cli`, `api-rest`, `api-gql`, `git-summary`, `git-lock`, and `plan-tooling` compile + tests pass with progress-enabled paths.
- E2E/manual:
  - Run a real CLI that uses `nils-term` and confirm stdout piping remains clean (progress appears only on stderr).

## Risks & gotchas
- `indicatif` output can be sensitive to draw targets and terminal-width detection. Mitigation: use fixed width + `to_writer` in tests.
- RAII + `Drop` can cause surprising output if a progress object is dropped without rendering. Mitigation: track “rendered at least once” and only emit finish output when needed.
- Some CI environments set pseudo-TTYs; relying on `is_terminal()` is usually fine but tests should not depend on it. Mitigation: tests should force draw target and “enabled” setting explicitly.
- Progress rendering on stderr can be disrupted by other stderr output (e.g. logging). Mitigation: keep progress usage scoped; avoid emitting logs while progress is actively updating (or route logs elsewhere for specific commands).
- Some commands intentionally inherit stderr from child processes (e.g. `semantic-commit` invoking `git commit`); progress must not render concurrently or output becomes unreadable.
- `git-scope` has a parity requirement; any progress output must be opt-in and must not change default stdout output.
- `NO_COLOR` / `TERM=dumb` environments may expect simplified output. Mitigation: document the behavior and rely on `indicatif`/terminal defaults rather than adding bespoke env-var parsing in v1.

## Rollback plan
- Remove `crates/nils-term` from workspace members and delete the crate directory.
- Remove `indicatif` from workspace dependencies.
- Revert the `cli-template` demo integration (and any references/imports).
- Revert any `Cargo.lock` changes introduced by adding dependencies.
