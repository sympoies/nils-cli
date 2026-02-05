# Plan: Linux support for screen-record (Ubuntu 24.04, X11 + ffmpeg)

## Overview
Extend the existing `screen-record` CLI to run on Linux, targeting Ubuntu 24.04. The macOS backend
remains unchanged (ScreenCaptureKit + AVFoundation), while Linux uses an X11 backend for deterministic
window discovery and delegates capture/encoding to `ffmpeg`. The CLI surface, output contract, and
deterministic `CODEX_SCREEN_RECORD_TEST_MODE=1` behavior stay backwards compatible.

## Scope
- In scope:
  - Linux runtime support on Ubuntu 24.04 for X11/Xorg sessions (and XWayland when `DISPLAY` is set).
  - Preserve the existing flat-flag CLI contract, selection rules, and stdout/stderr contract.
  - Implement Linux backends for:
    - `--list-windows`, `--list-apps`
    - recording mode (`--duration`, `--audio`, `--path`, `--format`)
    - screenshot mode (`--screenshot`, `--image-format`, `--dir`, `--path`)
    - `--preflight` as a Linux prerequisite check (ffmpeg + X11 availability)
    - `--request-permission` on Linux as an alias of `--preflight` (no OS permission gate; still validates prerequisites)
  - Add Linux-focused tests and CI coverage on GitHub Actions `ubuntu-24.04`.
  - Documentation updates (crate README + repo root README).
- Out of scope:
  - Wayland-native window capture without XWayland (no `DISPLAY`): portal/PipeWire implementations.
  - Full-screen or region capture modes (the tool remains “single window” focused).
  - Advanced audio device selection UI (multiple mics, per-app routing).
  - Windows support.

## Assumptions (if any)
1. “Ubuntu 24.04 support” means “works in an X11/Xorg session” (including CI via Xvfb); Wayland-only
   sessions will exit with a clear error and remediation guidance.
2. `ffmpeg` is considered a runtime prerequisite on Linux (installed via `apt-get install ffmpeg`).
3. Audio capture uses PulseAudio-compatible APIs via PipeWire (`pipewire-pulse`), queried through
   `pactl` when `--audio` is not `off`.

## Sprint 1: Spec + platform dispatch
**Goal**: Make Linux behavior explicit and refactor the runtime so macOS and Linux backends can coexist cleanly.
**Demo/Validation**:
- Command(s):
  - `cargo run -p screen-record -- --help | rg "Linux|macOS|X11|Wayland" || true`
  - `CODEX_SCREEN_RECORD_TEST_MODE=1 cargo test -p screen-record`
- Verify:
  - Help/README describe Linux prerequisites and limitations.
  - Test mode remains stable and continues to work cross-platform.

### Task 1.1: Update CLI + README contract to include Linux
- **Location**:
  - `crates/screen-record/README.md`
  - `README.md`
  - `crates/screen-record/src/cli.rs`
  - `completions/zsh/_screen-record`
  - `completions/bash/screen-record`
- **Description**: Update user-facing documentation and help strings to reflect cross-platform support:
  - README: change “macOS 12+ CLI” wording to “macOS 12+ and Linux (Ubuntu 24.04 X11)”.
  - Document Linux runtime prerequisites (`ffmpeg`, X11 session / `DISPLAY`) and the explicit limitation
    for Wayland-only sessions.
  - Clarify Linux semantics for `--preflight` (prerequisite check) and `--request-permission` (alias of `--preflight`).
  - Update clap `about` string to remove “on macOS” and keep it accurate for both platforms.
  - Update `--preflight` / `--request-permission` help strings (and Zsh completion descriptions) to be
    accurate cross-platform: macOS permission behavior vs Linux prerequisite checks.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - `crates/screen-record/README.md` includes a short “Linux (Ubuntu 24.04)” section with prerequisites,
    supported session types, and at least 2 runnable Linux examples.
  - Repo root `README.md` no longer describes `screen-record` as macOS-only.
  - `screen-record --help` no longer claims macOS-only behavior in the top-level description.
  - `completions/zsh/_screen-record` no longer describes `--preflight` as “permission” only.
  - `completions/bash/screen-record` no longer describes `--preflight` as “permission” only.
- **Validation**:
  - `rg -n "Linux|Ubuntu|X11|Wayland|ffmpeg" crates/screen-record/README.md`
  - `rg -n "screen-record" README.md | rg "Linux|Ubuntu|X11|ffmpeg"`
  - `rg -n "preflight|request-permission" completions/zsh/_screen-record`
  - `rg -n "preflight|request-permission" completions/bash/screen-record`

### Task 1.2: Introduce a platform backend interface and dispatch in `run`
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/lib.rs`
  - `crates/screen-record/src/error.rs`
- **Description**: Refactor `run(cli)` so platform behavior is isolated behind a small internal interface:
  - Keep `CODEX_SCREEN_RECORD_TEST_MODE=1` as the first/fast path (no OS checks).
  - Replace the current “non-macOS always usage error” guard with backend dispatch:
    - macOS backend: existing modules (`crate::macos::*`).
    - Linux backend: new modules (`crate::linux::*`).
    - Other OSes: usage error (exit 2) with a clear message listing supported OSes.
  - Ensure error messages remain user-facing (no stacks) and preserve exit code conventions.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - `cargo test -p screen-record` passes on macOS and Linux CI without relying on macOS APIs.
  - On unsupported OS targets, the binary still compiles and exits 2 with a clear message.
  - Existing mode/flag validation remains unchanged (selection rules, format conflicts, stdout contract).
- **Validation**:
  - `cargo test -p screen-record`
  - `cargo run -p screen-record -- --list-windows` (expected to work only under test mode or supported OS)

### Task 1.3: Define Linux preflight behavior and implement prerequisites check
- **Location**:
  - `crates/screen-record/src/linux/mod.rs`
  - `crates/screen-record/src/linux/preflight.rs`
  - `crates/screen-record/src/run.rs`
- **Description**: Implement Linux `--preflight` to validate prerequisites without performing capture:
  - Verify `ffmpeg` exists on `PATH` (use `nils_common::process::find_in_path`).
  - Verify X11 availability by checking `DISPLAY` and establishing an X11 connection (best-effort).
  - For Wayland-only sessions (`WAYLAND_DISPLAY` set and `DISPLAY` missing), return a runtime error with
    actionable guidance (log into “Ubuntu on Xorg”).
  - Keep stdout empty; emit human guidance on stderr only, matching existing contract.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - On Linux without `ffmpeg`, `--preflight` exits 1 and mentions how to install it.
  - On Wayland-only sessions, `--preflight` exits 1 and mentions `DISPLAY`/Xorg requirement.
  - On a valid X11 session with `ffmpeg`, `--preflight` exits 0 with empty stdout/stderr.
- **Validation**:
  - `cargo test -p screen-record -- cli_smoke`
  - Manual (Ubuntu): `./wrappers/screen-record --preflight; echo $?`

### Task 1.4: Define Linux `--request-permission` semantics and test coverage
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/linux/preflight.rs`
  - `crates/screen-record/tests/linux_request_permission.rs`
- **Description**: Implement Linux `--request-permission` as an alias of Linux `--preflight`:
  - Keep stdout empty; use stderr only for actionable guidance on failure.
  - Do not attempt any OS “permission request” behavior on Linux.
  - Add Linux-only tests that exercise failure cases without requiring a real X11 session:
    - missing `ffmpeg` on `PATH`
    - Wayland-only session (`WAYLAND_DISPLAY` set, `DISPLAY` unset)
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - On Linux without `ffmpeg`, `--request-permission` exits 1 and mentions how to install it.
  - On Wayland-only sessions, `--request-permission` exits 1 and mentions `DISPLAY`/Xorg requirement.
  - On Linux success, `--request-permission` exits 0 with empty stdout/stderr.
- **Validation**:
  - `cargo test -p screen-record -- linux_request_permission`

## Sprint 2: X11 window/app discovery (deterministic, scriptable)
**Goal**: Implement `--list-windows` / `--list-apps` and provide the selection inputs needed for recording/screenshot on Linux.
**Demo/Validation**:
- Command(s):
  - `./wrappers/screen-record --list-windows | head -n 20`
  - `./wrappers/screen-record --list-apps | head -n 20`
- Verify:
  - Output is TSV-only and deterministically sorted.
  - Window IDs printed by list output are accepted by `--window-id`.

### Task 2.1: Add Linux X11 module scaffolding and dependencies
- **Location**:
  - `crates/screen-record/Cargo.toml`
  - `crates/screen-record/src/linux/mod.rs`
  - `crates/screen-record/src/lib.rs`
- **Description**: Add a Linux-only module tree and dependencies:
  - Add `x11rb` (or an equivalent pure-Rust X11 client) under `cfg(target_os = "linux")`.
  - Add `crate::linux` module export behind `cfg(target_os = "linux")`.
  - Ensure non-Linux/non-macOS builds remain compile-only with clear runtime errors.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo build --workspace` succeeds on macOS and Linux CI.
  - Linux code is fully `cfg`-gated (no X11 deps compiled on macOS).
- **Validation**:
  - `cargo build -p screen-record`
  - `cargo tree -p screen-record | rg "x11rb" || true`

### Task 2.2: Implement X11 shareable content snapshot (windows + metadata)
- **Location**:
  - `crates/screen-record/src/linux/x11.rs`
  - `crates/screen-record/src/linux/mod.rs`
  - `crates/screen-record/src/types.rs`
- **Description**: Implement a Linux X11 equivalent of “shareable content”:
  - Connect to the X server using `DISPLAY`.
  - Enumerate top-level client windows using `_NET_CLIENT_LIST` and stacking order using
    `_NET_CLIENT_LIST_STACKING` when available.
  - When EWMH client list atoms are missing (common under Xvfb without a window manager), fall back to
    `QueryTree` on the root window and treat mapped, viewable child windows as candidates.
  - For each window, populate `WindowInfo`:
    - `id`: X11 window id (XID) as `u32` printed in decimal in TSV.
    - `owner_name`: derived from `WM_CLASS` (prefer class part) with a stable fallback.
    - `title`: from `_NET_WM_NAME` (UTF-8) with fallback to `WM_NAME`.
    - `bounds`: absolute window geometry in pixels.
    - `active`: match `_NET_ACTIVE_WINDOW`.
    - `on_screen`: treat as true when viewable/mapped and not minimized (`_NET_WM_STATE_HIDDEN`).
    - `owner_pid`: from `_NET_WM_PID` when present; otherwise `0`.
    - `z_order`: derived from stacking list index (frontmost should be lowest `z_order`). When stacking order is unavailable,
      derive from the fallback window list order (assign the last window `z_order=0`, then increment).
  - Ensure all string fields are UTF-8 safe and default to empty string when missing.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - On Ubuntu X11, `--list-windows` returns at least one row on a typical desktop.
  - Under Xvfb without a window manager, a mapped window created by a test process is discoverable via the fallback enumeration path.
  - The returned `WindowInfo` values are sufficient for existing selection logic (`--active-window`,
    `--app`, frontmost selection by `z_order`).
  - Missing X11 properties do not crash the process; they degrade to safe defaults.
- **Validation**:
  - Manual (Ubuntu X11): `./wrappers/screen-record --list-windows | head -n 20`
  - CI: `cargo test -p screen-record` (compilation + unit tests)

### Task 2.3: Implement Linux `--list-apps` from window snapshot
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/linux/x11.rs`
- **Description**: Provide `--list-apps` output on Linux by deriving apps from the window snapshot:
  - Group windows by `(owner_name, owner_pid)` and emit one row per unique pair.
  - Output contract stays identical: TSV columns `app_name`, `pid`, `bundle_id`.
  - On Linux, `bundle_id` is always empty.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `--list-apps` is deterministic (sorted by `app_name`, then `pid`).
  - Output is TSV-only with no header and no extra prose.
- **Validation**:
  - Manual (Ubuntu X11): `./wrappers/screen-record --list-apps | head -n 20`
  - `CODEX_SCREEN_RECORD_TEST_MODE=1 cargo test -p screen-record -- recording_test_mode`

### Task 2.4: Update non-macOS runtime behavior tests for Linux support
- **Location**:
  - `crates/screen-record/tests/non_macos.rs`
- **Description**: Adjust tests so they remain correct after Linux support is added:
  - Restrict the existing “non-macOS exits 2” assertion to OSes that remain unsupported (e.g. Windows).
  - Add a Linux-only test that asserts `--preflight` provides actionable errors when prerequisites are missing
    (use PATH stubs to simulate missing `ffmpeg`).
- **Dependencies**:
  - Task 1.3
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `cargo test -p screen-record` passes on Linux CI and macOS.
  - The test suite no longer encodes “Linux is unsupported” assumptions.
- **Validation**:
  - `cargo test -p screen-record -- non_macos`

## Sprint 3: Linux capture implementation (ffmpeg)
**Goal**: Implement recording and screenshot on Linux using `ffmpeg`, preserving the stdout/stderr contract.
**Demo/Validation**:
- Command(s):
  - `./wrappers/screen-record --active-window --duration 2 --audio off --path ./recordings/active.mp4`
  - `./wrappers/screen-record --screenshot --active-window --path ./screenshots/active.png`
- Verify:
  - On success, stdout prints only the resolved output path.
  - Output files exist and are non-empty.

### Task 3.1: Implement `ffmpeg` runner for recording (video-only)
- **Location**:
  - `crates/screen-record/src/linux/ffmpeg.rs`
  - `crates/screen-record/src/linux/mod.rs`
  - `crates/screen-record/src/run.rs`
- **Description**: Implement the Linux recording pipeline by spawning `ffmpeg`:
  - Use X11 capture targeting the selected window id (pass `-window_id` using hex formatting).
  - Respect `--duration` via `ffmpeg -t N` (N is the CLI duration seconds) and return when the process exits.
  - Invoke `ffmpeg` with `-hide_banner -loglevel error -nostdin -y` to keep stderr quiet on success and avoid interactive prompts.
  - Encode using H.264 for video and choose container based on existing `ContainerFormat` resolution.
  - Map `ffmpeg` failures to a runtime error (exit 1) with stderr surfaced succinctly (avoid dumping megabytes).
  - Ensure Ctrl-C stops the capture gracefully (terminate child process and wait).
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - On Ubuntu X11, recording a window for N seconds produces a playable `.mp4` or `.mov`.
  - `--format` vs `--path` extension conflict rules remain enforced by existing code.
  - A missing `ffmpeg` binary produces a clear runtime error (exit 1) pointing to installation.
  - On success, stdout prints only the output path and stderr remains empty (no ffmpeg banner/progress noise).
- **Validation**:
  - Manual (Ubuntu X11): `./wrappers/screen-record --active-window --duration 1 --audio off --path ./recordings/active.mp4`
  - `CODEX_SCREEN_RECORD_TEST_MODE=1 cargo test -p screen-record -- recording_test_mode`

### Task 3.2: Implement `ffmpeg` runner for screenshots (single frame)
- **Location**:
  - `crates/screen-record/src/linux/ffmpeg.rs`
  - `crates/screen-record/src/run.rs`
- **Description**: Implement Linux screenshot capture via `ffmpeg` using the selected window id:
  - Capture exactly one frame and write to the resolved image path.
  - Invoke `ffmpeg` with `-hide_banner -loglevel error -nostdin -y` to keep stderr quiet on success and avoid interactive prompts.
  - Support `png` and `jpg` via `ffmpeg` encoders; support `webp` when available and provide a clear
    runtime error when encoding fails.
  - Preserve existing screenshot path/default naming logic and mode rules.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - On Ubuntu X11, `--screenshot --active-window` produces a non-empty image file.
  - The stdout contract matches macOS: print only the final absolute output path with a trailing newline.
  - `--image-format` conflict behavior remains unchanged.
  - On success, stderr remains empty (no ffmpeg banner/progress noise).
- **Validation**:
  - Manual (Ubuntu X11): `./wrappers/screen-record --screenshot --active-window --path ./screenshots/active.png`
  - `CODEX_SCREEN_RECORD_TEST_MODE=1 cargo test -p screen-record -- recording_test_mode`

### Task 3.3: Implement Linux audio source resolution (`mic`, `system`, `both`)
- **Location**:
  - `crates/screen-record/src/linux/audio.rs`
  - `crates/screen-record/src/linux/mod.rs`
  - `crates/screen-record/src/run.rs`
- **Description**: Add Linux audio input selection logic compatible with Ubuntu 24.04 (PipeWire + Pulse):
  - `--audio mic`: capture from the default source.
  - `--audio system`: capture from the default sink monitor source (`SINK_NAME.monitor`).
  - `--audio both`: capture two audio tracks (system + mic) and enforce the existing `.mov` restriction.
  - Use `pactl info` and `pactl get-default-sink` / `pactl get-default-source` (or `pactl list` fallback)
    to resolve names; provide clear runtime errors when `pactl` is missing or sources cannot be resolved.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 8
- **Acceptance criteria**:
  - On Ubuntu 24.04, `--audio system` and `--audio mic` start capture without requiring extra flags.
  - When audio prerequisites are missing, exit 1 with an actionable message (install pipewire-pulse/pulseaudio-utils).
  - `--audio both` continues to require `.mov` with a consistent usage error (exit 2) across platforms.
- **Validation**:
  - Manual (Ubuntu X11): `./wrappers/screen-record --active-window --duration 1 --audio mic --path ./recordings/mic.mov`
  - Unit (Linux CI): `cargo test -p screen-record -- audio`

### Task 3.4: Integrate audio inputs into `ffmpeg` invocation and validate stream mapping
- **Location**:
  - `crates/screen-record/src/linux/ffmpeg.rs`
  - `crates/screen-record/src/run.rs`
- **Description**: Extend the `ffmpeg` command builder to include audio inputs and stable mapping:
  - `off`: video-only output.
  - `system` / `mic`: single audio track with AAC (or a default widely supported codec).
  - `both`: two audio tracks; keep the `.mov` restriction and ensure output contains both tracks.
  - Keep stderr handling bounded; surface the most relevant error snippet on failure.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 7
- **Acceptance criteria**:
  - `--audio system` and `--audio mic` produce files that include an audio stream.
  - `--audio both` produces two audio streams in a `.mov` output and fails with exit 2 for `.mp4`.
  - Interrupting capture (Ctrl-C) still results in a valid container file when possible.
- **Validation**:
  - Manual (Ubuntu X11): `./wrappers/screen-record --active-window --duration 2 --audio system --path ./recordings/system.mov`
  - Manual (Ubuntu X11): `ffprobe -hide_banner -show_streams ./recordings/system.mov | rg "codec_type=audio" || true`

## Sprint 4: Linux tests + CI hardening
**Goal**: Add automated coverage for the Linux backend and ensure CI is deterministic on Ubuntu 24.04.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/screen-record-linux-ubuntu-2404-plan.md`
  - `xvfb-run -a ./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify:
  - Linux-specific logic is covered by unit/integration tests and runs in CI.

### Task 4.1: Add Linux unit tests for `ffmpeg` argument building and audio resolution
- **Location**:
  - `crates/screen-record/src/linux/ffmpeg.rs`
  - `crates/screen-record/src/linux/audio.rs`
  - `crates/screen-record/tests/linux_unit.rs`
- **Description**: Add Linux-focused tests that do not require a real desktop:
  - Unit-test that the `ffmpeg` command contains the expected flags (`-window_id`, `-t`, container output).
  - Unit-test audio source resolution parsing by stubbing `pactl` via `StubBinDir` and capturing invoked args.
  - Ensure tests are gated to Linux via `#[cfg(target_os = "linux")]`.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests cover at least: `off`, `system`, `mic`, and `both` command construction.
  - Tests validate the `.mov` restriction for `both` is enforced at the CLI level.
  - Tests do not require `ffmpeg` or `pactl` installed (they use stubs).
- **Validation**:
  - `cargo test -p screen-record -- linux_unit`

### Task 4.2: Add X11 integration tests under Xvfb and wire into CI
- **Location**:
  - `crates/screen-record/tests/linux_x11_integration.rs`
  - `.github/workflows/ci.yml`
- **Description**: Add an integration test that exercises the Linux X11 backend end-to-end in CI:
  - Run tests under Xvfb (`xvfb-run -a`) on GitHub Actions `ubuntu-24.04`.
  - Create a minimal X11 window via `x11rb` in the test process with a known title/class.
  - Verify `screen-record --list-windows` returns the created window (without assuming a window manager is present)
    and that `--window-id` selection resolves it.
  - Stub `ffmpeg` so recording/screenshot commands complete quickly and deterministically (the stub should
    create the output file and record received args for assertions).
  - Keep the test hermetic (no dependence on host desktop state).
- **Dependencies**:
  - Task 2.2
  - Task 3.2
  - Task 4.1
- **Complexity**: 9
- **Acceptance criteria**:
  - GitHub Actions `runs-on` is pinned to `ubuntu-24.04` for both the `test` and `coverage` jobs.
  - CI installs `xvfb` and runs both the `test` and `coverage` jobs under `xvfb-run -a` (for any steps that execute tests).
  - The integration test asserts that Linux backend uses X11 window ids and passes them to `ffmpeg` via `-window_id`.
  - The test suite remains fast (no real encoding) and stable on GitHub Actions.
- **Validation**:
  - `rg -n "xvfb-run|xvfb" .github/workflows/ci.yml`
  - Manual (Ubuntu): `xvfb-run -a cargo test -p screen-record -- linux_x11_integration`
  - CI run on a PR (expected): `ubuntu-24.04` jobs pass

### Task 4.3: Update docs for Linux caveats and troubleshooting
- **Location**:
  - `crates/screen-record/README.md`
  - `crates/screen-record/src/run.rs`
- **Description**: Add concise troubleshooting guidance for Ubuntu 24.04:
  - Wayland-only session errors: how to switch to “Ubuntu on Xorg”.
  - Wayland sessions with XWayland: only X11 client windows are capturable; for Wayland-native apps, switch to Xorg.
  - Missing runtime prerequisites: `ffmpeg` and `pactl` install commands.
  - Common capture pitfalls (minimized windows, occlusion in X11 capture).
  - Ensure runtime errors mention the exact missing tool name when possible.
- **Dependencies**:
  - Task 3.4
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - README includes a “Troubleshooting (Linux)” section with 3–5 targeted entries.
  - Error messages referenced by docs match implementation (no stale wording).
- **Validation**:
  - `rg -n "Troubleshooting|Wayland|Xorg|ffmpeg|pactl" crates/screen-record/README.md`

## Testing Strategy
- Unit:
  - Keep selection logic tests as-is (`crates/screen-record/src/select.rs`).
  - Add Linux unit tests for `ffmpeg` argument construction and `pactl` parsing (`Task 4.1`).
- Integration:
  - Linux X11 integration under Xvfb with stubbed `ffmpeg` (`Task 4.2`).
  - Preserve deterministic `CODEX_SCREEN_RECORD_TEST_MODE=1` integration tests as cross-platform baseline.
- E2E/manual:
  - On Ubuntu 24.04 Xorg: record `--active-window` for 2–3 seconds and verify output with `ffplay` or `mpv`.
  - Validate audio modes using `ffprobe` to confirm audio streams are present.

## Risks & gotchas
- Wayland default: Ubuntu 24.04 uses Wayland by default; requiring Xorg is a product tradeoff and must be clearly messaged.
- XWayland limitations: on Wayland, `DISPLAY` may be set but only X11 client windows are discoverable/capturable; docs should steer users to Xorg.
- Window IDs: X11 XIDs are commonly displayed in hex elsewhere; this CLI prints decimal, so docs and errors must be clear.
- Geometry correctness: window frames vs client area can differ (decorations); capture should prefer the full window surface.
- Audio source resolution: default sink/source naming differs across PipeWire/Pulse setups; parsing must be resilient.
- `ffmpeg` stderr noise: raw stderr can be huge; errors should be trimmed to relevant lines.

## Rollback plan
- Keep macOS backend untouched and gated by `cfg(target_os = "macos")`.
- If Linux runtime proves unstable, reintroduce a Linux runtime guard to exit 2 on Linux and keep CI green by retaining deterministic test mode tests.
- Revert CI Xvfb wiring if it causes flakiness; Linux integration tests can be temporarily gated behind an env var while fixes land.
