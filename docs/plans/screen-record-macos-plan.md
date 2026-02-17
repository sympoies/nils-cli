# Plan: macOS screen-record (Rust + ScreenCaptureKit, window/app + audio)

## Overview
This plan adds a new Rust CLI, `screen-record`, to the `nils-cli` workspace. The CLI records a single
macOS 12+ window (or a single app via window selection) into a video file using ScreenCaptureKit for
capture and AVFoundation for encoding/muxing. The primary target user is Codex workflows (capture a
repeatable artifact to a user-provided path), but it should also be useful for humans to record and
share step-by-step demos.

## Scope
- In scope:
  - New workspace crate + binary: `crates/screen-record` → `screen-record`
  - macOS 12+ implementation using ScreenCaptureKit + AVFoundation
  - Record modes:
    - `--window-id` to record a specific window
    - `--app` and `--window-name` to resolve a single window to record (deterministic selection rules)
    - `--list-windows` to print selectable windows with stable, parseable output
  - Audio modes:
    - system audio (from ScreenCaptureKit)
    - microphone audio (from AVFoundation capture) with `--audio mic` / `--audio both`
  - Output:
    - `--path` output file path (required for recording)
    - container: `.mov` (default) and `.mp4` (when requested by extension)
  - Deterministic test mode for CI and unit/integration tests (no real OS capture required)
  - Shell completions: Zsh + Bash
- Out of scope:
  - Real-time streaming API (pipe frames/audio to another process)
  - Multi-window compositing or “record an entire app with all windows merged”
  - Video editing features (trimming, annotations, overlays)
  - Cross-platform runtime support (non-macOS builds will compile but exit with a clear error)
  - Advanced device selection UI (multiple microphones, per-app audio routing)

## Assumptions (if any)
1. Target OS is macOS 12+ (Monterey or newer); Apple Silicon is the primary validation target.
2. Screen Recording and Microphone permissions are granted by the user via System Settings.
3. “Record a single app” means “resolve to one concrete window and record that window” (not a composed multi-window capture).
4. The “both” audio mode is implemented as two audio tracks (system + mic) and requires `.mov` output; real-time mixing is explicitly out of scope for v1.

## Sprint 1: Spec + crate scaffold (compile everywhere)
**Goal**: Make the UX contract explicit and land a new crate that compiles on all workspace targets.
**Demo/Validation**:
- Command(s): `cargo run -p screen-record -- --help`
- Verify: help text documents flags, exit codes, and macOS-only behavior.

### Task 1.1: Write spec (CLI, selection rules, exit codes, artifacts)
- **Location**:
  - `crates/screen-record/README.md`
- **Description**: Define the `screen-record` CLI contract: flags, selection rules (`--app`, `--window-name`, `--window-id`),
  output paths, exit codes (0 success, 1 runtime failure, 2 usage error), and the stdout/stderr contract
  (stdout prints only the output file path on success). Document deterministic test mode env vars.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - README includes a full flag table and at least 5 runnable examples (list windows, record app, record window-id, duration, audio mode).
  - README documents ambiguous selection behavior (multiple matches) and the exact error text style.
  - README explicitly defines how `.mov` vs `.mp4` selection works (by extension or explicit flag).
  - README defines the exact `--list-windows` / `--list-apps` output format (TSV) and column order.
- **Validation**:
  - `rg \"screen-record\" crates/screen-record/README.md`
  - `rg \"exit codes\" -n crates/screen-record/README.md`

### Task 1.2: Add crate skeleton and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/screen-record/Cargo.toml`
  - `crates/screen-record/src/main.rs`
  - `crates/screen-record/src/cli.rs`
  - `crates/screen-record/src/lib.rs`
  - `crates/screen-record/src/macos/mod.rs`
- **Description**: Create `crates/screen-record` as a new binary crate and add it to the workspace members.
  Implement clap parsing in `src/cli.rs` and a stub runner that:
  - on macOS: prints a “not implemented yet” usage error (exit 2) behind a feature flag or temporary code path
  - on non-macOS: prints “macOS 12+ only” and exits 2
  The goal is to keep the workspace build green from day one.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `cargo build --workspace` succeeds on the current machine.
  - `cargo run -p screen-record -- --help` exits 0.
  - On non-macOS targets, `screen-record` compiles and exits 2 with a clear message.
- **Validation**:
  - `cargo run -p screen-record -- --help`
  - `cargo test -p screen-record -- --help` (ensures the crate is wired and test harness is present)

### Task 1.3: Link macOS frameworks safely (build.rs + cfg gating)
- **Location**:
  - `crates/screen-record/build.rs`
  - `crates/screen-record/Cargo.toml`
- **Description**: Add macOS-only linking for frameworks used by the implementation (ScreenCaptureKit, AVFoundation,
  CoreMedia, CoreVideo, Foundation, CoreGraphics) via `build.rs`, guarded by `cfg(target_os = \"macos\")`.
  Ensure non-macOS builds do not attempt to link Apple frameworks.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo build -p screen-record` succeeds on macOS with the build script enabled.
  - Non-macOS builds (CI) do not reference Apple frameworks at link time.
- **Validation**:
  - `cargo build -p screen-record -vv | rg \"ScreenCaptureKit|AVFoundation\"`

## Sprint 2: Window discovery + selection (macOS + deterministic test mode)
**Goal**: Implement `--list-windows` / `--list-apps` and robust window/app selection logic without yet recording video.
**Demo/Validation**:
- Command(s): `cargo run -p screen-record -- --list-windows | head`
- Verify: output includes window id, owner/app, title, and bounds.

### Task 2.1: Implement macOS permission preflight and request flags
- **Location**:
  - `crates/screen-record/src/macos/permissions.rs`
  - `crates/screen-record/src/macos/mod.rs`
  - `crates/screen-record/src/main.rs`
- **Description**: Add a macOS permission helper that checks Screen Recording permission and can best-effort guide users to grant it.
  Implement CLI flags:
  - `--preflight` to check and return a helpful error if permission is missing
  - `--request-permission` as best-effort: attempt a system request when possible, otherwise open the
    System Settings privacy pane for Screen Recording, then re-check and report status
  Errors must be actionable and mention the System Settings path.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - With permission granted, `--preflight` exits 0 and prints nothing to stdout.
  - With permission missing, `--preflight` exits 1 and prints a single actionable error to stderr.
  - `--request-permission` is explicitly documented as best-effort and reports success/failure with a clear next step.
- **Validation**:
  - `cargo run -p screen-record -- --preflight`

### Task 2.2: Implement ScreenCaptureKit shareable content snapshot (windows + apps)
- **Location**:
  - `crates/screen-record/Cargo.toml`
  - `crates/screen-record/src/macos/shareable.rs`
  - `crates/screen-record/src/macos/mod.rs`
- **Description**: Wrap the minimal ScreenCaptureKit calls needed to fetch `SCShareableContent` and extract a
  Rust snapshot list of windows (id, owner name, title, bounds, on-screen flag) and apps (pid, name, bundle id).
  The wrapper should hide Objective-C runtime details behind a small Rust API, concentrating `unsafe`
  Objective-C bridging in `macos/ffi.rs` (using an `objc2`-style approach and blocks where required).
- **Dependencies**:
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Fetching shareable content returns at least one window on a typical macOS desktop.
  - The wrapper is resilient to missing titles and missing bundle ids.
  - The wrapper provides deterministic sorting (by owner, then title, then id) for stable output.
  - `--list-windows` output is one record per line in a fixed, parseable format (TSV with a documented column order).
- **Validation**:
  - `cargo run -p screen-record -- --list-windows | head -n 20`

### Task 2.3: Implement selection rules (`--window-id`, `--app`, `--window-name`, `--active-window`)
- **Location**:
  - `crates/screen-record/src/select.rs`
  - `crates/screen-record/src/cli.rs`
  - `crates/screen-record/src/main.rs`
- **Description**: Implement deterministic selection logic:
  - `--window-id N` selects exactly that window id (error if missing).
  - `--app NAME` filters windows by owner/app name (case-insensitive substring), then:
    - if `--window-name` is provided: further filter by title substring
    - else: select the frontmost on-screen window for that app when possible; otherwise require disambiguation (error with candidates).
  - `--active-window` selects the single frontmost window on the current Space.
  Ensure ambiguous matches return exit 2 with a list of candidate window ids.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Ambiguous selection exits 2 and prints candidates in the same format as `--list-windows`.
  - Selection errors never print a stack trace; stderr is user-facing.
  - Selection logic is unit-tested with an in-memory window list.
- **Validation**:
  - `cargo test -p screen-record -- select`

### Task 2.4: Add `--list-apps` (stable, parseable output)
- **Location**:
  - `crates/screen-record/src/cli.rs`
  - `crates/screen-record/src/main.rs`
- **Description**: Add `--list-apps` to print selectable applications from the same shareable content snapshot,
  in a stable, parseable format (one app per line; prefer TSV with a fixed column order). This is intended
  to make `--app` selection discoverable without requiring window titles.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `--list-apps` output is deterministic (sorted by app name, then pid).
  - Output includes app name, pid, and bundle id when available.
  - Output is one record per line and does not include extra prose.
- **Validation**:
  - `cargo run -p screen-record -- --list-apps | head -n 20`

### Task 2.5: Add deterministic test mode (fake windows + fake recordings)
- **Location**:
  - `crates/screen-record/src/test_mode.rs`
  - `crates/screen-record/src/main.rs`
  - `crates/screen-record/tests/test_mode_cli.rs`
  - `crates/screen-record/tests/fixtures/sample.mov`
  - `crates/screen-record/tests/fixtures/sample.mp4`
- **Description**: Add `AGENTS_SCREEN_RECORD_TEST_MODE=1` to bypass macOS APIs and return deterministic,
  in-memory shareable content and deterministic “recording” output (copy `tests/fixtures/sample.mov` to `--path`,
  selecting fixture by extension when `.mp4` paths are used).
  This enables CI tests on non-macOS without requiring Screen Recording permission.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - With test mode enabled, `--list-windows` prints a stable fixture window list.
  - With test mode enabled, `--app Terminal --duration 1 --path out.mov` produces a non-empty file and prints its path.
  - With test mode enabled, `.mp4` output paths are supported for tests that exercise container selection.
  - Tests run on any OS and do not require external binaries.
- **Validation**:
  - `AGENTS_SCREEN_RECORD_TEST_MODE=1 cargo test -p screen-record`

## Sprint 3: Record video to file (ScreenCaptureKit + AVAssetWriter)
**Goal**: Implement end-to-end video recording for a selected window, with duration control and clean finalization.
**Demo/Validation**:
- Command(s): `cargo run -p screen-record -- --app Terminal --duration 3 --audio off --path \"./recordings/terminal-video.mov\"`
- Verify: output file exists and is playable in QuickTime Player.

### Task 3.1: Implement AVAssetWriter wrapper (video track)
- **Location**:
  - `crates/screen-record/src/macos/writer.rs`
  - `crates/screen-record/src/macos/mod.rs`
- **Description**: Implement a small wrapper around AVFoundation’s `AVAssetWriter` that can:
  - create a writer for `.mov` and `.mp4` based on output extension
  - create a video input configured for H.264 (baseline/main) and append video `CMSampleBuffer` frames
  - finish writing reliably on stop (flush + close file)
  The wrapper must support “start session at first frame timestamp” to keep A/V sync correct later.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Writer creates a non-empty file when fed synthetic frames in test mode.
  - Writer surfaces meaningful errors (file permission, unsupported extension, encoder failure).
- **Validation**:
  - `AGENTS_SCREEN_RECORD_TEST_MODE=1 cargo test -p screen-record -- writer`

### Task 3.2: Implement ScreenCaptureKit stream output (video sample buffers)
- **Location**:
  - `crates/screen-record/src/macos/stream.rs`
  - `crates/screen-record/src/macos/mod.rs`
- **Description**: Implement an `SCStream` runner for window capture:
  - configure capture size and frame rate defaults (e.g., 30fps)
  - register an output callback for `.screen` sample buffers
  - forward buffers to the writer on a single, serialized queue
  Implement a clean stop path that ensures the stream stops before finalizing the writer.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 9
- **Acceptance criteria**:
  - Recording for N seconds produces a playable file with roughly N seconds duration.
  - Stopping early (SIGINT) produces a finalized file (no corruption).
  - Errors include enough context to debug (selected window id, config, writer state).
- **Validation**:
  - `cargo run -p screen-record -- --active-window --duration 2 --audio off --path \"./recordings/active-window.mov\"`

### Task 3.3: Wire CLI runner (duration, output path resolution, stdout contract)
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/main.rs`
- **Description**: Implement the high-level command flow:
  - resolve output path (require `--path`, resolve relative paths under CWD)
  - validate argument combinations (mutually exclusive flags)
  - select window (Sprint 2) then record (Sprint 3)
  - print only the final output path to stdout on success
  - ensure stderr remains user-facing logs/errors
- **Dependencies**:
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Invalid flag combinations exit 2 with a single clear message.
  - Successful runs print exactly one line to stdout: the output path.
  - Missing `--path` exits 2 with a clear usage error.
- **Validation**:
  - `AGENTS_SCREEN_RECORD_TEST_MODE=1 cargo run -p screen-record -- --app Terminal --duration 1 --audio off --path \"./recordings/test.mov\"`

## Sprint 4: Audio (system + microphone) + polish
**Goal**: Add audio capture modes and ship a polished CLI with stable completions and tests.
**Demo/Validation**:
- Command(s): `cargo run -p screen-record -- --app Terminal --duration 5 --audio both --path \"./recordings/terminal-audio.mov\"`
- Verify: audio is present and the file plays in QuickTime Player.

### Task 4.1: Add system audio capture (ScreenCaptureKit audio output)
- **Location**:
  - `crates/screen-record/src/macos/stream.rs`
  - `crates/screen-record/src/macos/writer.rs`
  - `crates/screen-record/src/cli.rs`
- **Description**: Add `--audio system|off` support by enabling ScreenCaptureKit audio capture and writing
  audio sample buffers into the same file via a dedicated audio writer input (AAC).
  Ensure timestamps align to the same session start time as the video track.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 9
- **Acceptance criteria**:
  - `--audio system` produces a file with an audio track that plays back in QuickTime Player.
  - `--audio off` continues to produce video-only files with no audio track.
  - A/V sync is acceptable for short captures (no obvious drift over 30 seconds).
- **Validation**:
  - `cargo run -p screen-record -- --active-window --duration 3 --audio system --path \"./recordings/system-audio.mov\"`

### Task 4.2: Add microphone capture (AVFoundation capture session)
- **Location**:
  - `crates/screen-record/src/macos/mic.rs`
  - `crates/screen-record/src/macos/writer.rs`
  - `crates/screen-record/src/cli.rs`
- **Description**: Implement microphone capture as a separate CMSampleBuffer source using AVFoundation
  (e.g., an `AVCaptureSession` with an audio data output). Add `--audio mic|both`:
  - `mic`: mic track only
  - `both`: system audio + mic as two audio tracks (no mixing in v1; documented)
  For `--audio both`, require `.mov` output (error on `.mp4` with a clear message).
  Ensure permissions are handled with a clear error if microphone access is denied.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 10
- **Acceptance criteria**:
  - `--audio mic` produces a file with mic audio present.
  - `--audio both` produces a `.mov` file that contains both audio tracks according to the documented behavior.
  - `--audio both` with a `.mp4` path exits 2 with an actionable message.
  - Permission denial is reported as a runtime error (exit 1) with an actionable message.
- **Validation**:
  - `cargo run -p screen-record -- --active-window --duration 3 --audio mic --path \"./recordings/mic.mov\"`

### Task 4.3: Add completions (Zsh + Bash) and finalize docs
- **Location**:
  - `completions/zsh/_screen-record`
  - `completions/bash/screen-record`
  - `crates/screen-record/README.md`
- **Description**: Add shell completion files that match the final CLI flags. Update the README examples
  to include the final audio modes and window selection flows. Ensure completion tests still pass.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Zsh completion offers flags and value choices (notably `--audio`).
  - Bash completion covers the same flags at minimum.
  - README examples run as written on macOS.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.4: Add workspace-grade tests + required checks gate
- **Location**:
  - `crates/screen-record/tests/cli_smoke.rs`
  - `crates/screen-record/tests/non_macos.rs`
  - `crates/screen-record/tests/selection.rs`
  - `crates/screen-record/tests/recording_test_mode.rs`
- **Description**: Add deterministic integration tests using `AGENTS_SCREEN_RECORD_TEST_MODE=1` to cover:
  - flag parsing and validation errors (exit 2)
  - selection logic edge cases and ambiguity reporting
  - file output creation + stdout contract
  Ensure the crate contributes to workspace coverage and does not break CI on Linux.
- **Dependencies**:
  - Task 4.3
- **Complexity**: 7
- **Acceptance criteria**:
  - `cargo test -p screen-record` passes and does not require Screen Recording permission.
  - Non-macOS CI verifies default behavior is “macOS only” (exit 2) when test mode is not enabled.
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh` passes.
  - Workspace line coverage remains >= 80.00% when running the documented coverage commands.
- **Validation**:
  - `AGENTS_SCREEN_RECORD_TEST_MODE=1 cargo test -p screen-record`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`

## Testing Strategy
- Unit:
  - Selection/filtering logic over an in-memory window list (`crates/screen-record/src/select.rs`)
  - CLI argument validation and output path resolution (`crates/screen-record/src/cli.rs`, `crates/screen-record/src/run.rs`)
- Integration:
  - Deterministic test-mode end-to-end runs (writes a fixture `.mov`/`.mp4`, asserts stdout/stderr and exit codes)
  - Non-macOS behavior: compile + run tests asserting “macOS only” exit code and message (test mode bypasses this)
- E2E/manual:
  - macOS local run: record 5 seconds of Terminal by app name and confirm playback in QuickTime Player
  - Permission scenarios: run `--preflight` and `--request-permission` to confirm messaging

## Risks & gotchas
- ScreenCaptureKit and AVFoundation are Objective-C APIs; Rust bindings require careful lifetime and threading handling.
- Permissions are the #1 source of flaky behavior; error messages must be explicit and actionable.
- A/V sync and timestamp handling can be subtle; the writer should start session at the first observed timestamp and normalize subsequent buffers.
- “Both” audio mode (system + mic) may require either multi-track muxing or real-time mixing; mixing increases complexity materially.
- CI likely runs on Linux; macOS-only code must be behind `cfg(target_os = \"macos\")` and test mode must keep coverage healthy.

## Rollback plan
- Revert the new workspace member (`crates/screen-record`) and any completion files.
- Keep the existing Python/Swift `screenshot` skill unchanged; this plan introduces a new capability rather than replacing the current one.
- If the Codex integration script is added later, revert it to the prior behavior (or remove the new entrypoint).
