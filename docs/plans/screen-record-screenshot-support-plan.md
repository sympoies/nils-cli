# Plan: Add screenshot mode to screen-record (png/jpg/webp + default naming)

## Overview
Add a new screenshot capture mode to the existing `screen-record` CLI so it can output a single
window capture as `png`, `jpg`, or `webp`, while preserving the current video recording behavior
(`mov`/`mp4`) and all existing modes (`--list-*`, `--preflight`, `--request-permission`). Screenshot
mode will reuse the same window selection rules (`--window-id`, `--active-window`, `--app` +
`--window-name`) and will default to a timestamp-based filename that also includes resolved window
identity (id/owner/title) unless the user supplies an explicit output path.

## Scope
- In scope:
  - New screenshot mode flag: `--screenshot`.
  - Output image formats: `png`, `jpg`, `webp` (format selected by `--image-format` or `--path` extension).
  - Default naming: timestamp + resolved window identity (id/owner/title) with safe sanitization and collision handling.
  - Reuse existing selection logic and permission gates.
  - Deterministic `AGENTS_SCREEN_RECORD_TEST_MODE=1` support for screenshots (fixture copy), including integration tests.
  - Update docs and shell completions (Zsh + Bash) for new flags.
- Out of scope:
  - Full-screen / region selection capture.
  - Multi-window compositing or multi-monitor capture.
  - Animated outputs (GIF/APNG) or video-from-frames pipelines.
  - Advanced capture controls (cursor toggle, shadow toggle, scaling overrides) for v1.
  - Rich image tuning flags (quality/lossless knobs) for v1 (may be added later if needed).

## Assumptions (if any)
1. Backwards compatibility matters: keep the current flat-flag CLI surface (no subcommands) and make screenshot mode additive.
2. Screenshot capture requires Screen Recording permission on macOS (same as recording).
3. When `--path` is not provided in screenshot mode, output goes to `./screenshots/` under the current working directory.
4. Default screenshot format is `png` unless inferred from `--path` extension or overridden by `--image-format`.
5. WebP encoding should be implemented without requiring external tools on the happy path; if OS-level encoding is unavailable,
   fall back to an optional external encoder (with a clear error when missing) rather than silently producing the wrong format.

## Sprint 1: UX spec + CLI surface (no macOS capture yet)
**Goal**: Lock the screenshot UX contract (flags, mode rules, output contract) and wire the new mode into clap + validation without changing existing behavior.
**Demo/Validation**:
- Command(s):
  - `cargo run -p screen-record -- --help | rg \"--screenshot|--image-format|--dir\"`
  - `cargo test -p screen-record -- cli_smoke`
- Verify:
  - Help text documents screenshot flags.
  - Existing help/flag checks still pass.

### Task 1.1: Decide WebP encoding backend and failure behavior
- **Location**:
  - `crates/screen-record/src/macos/` (new module notes in code comments)
  - `docs/plans/screen-record-screenshot-support-plan.md`
- **Description**: Validate whether macOS 12+ can encode WebP via system frameworks and lock the product behavior:
  - Prototype encoding a small in-memory image to WebP via ImageIO/UTType.
  - If supported: use ImageIO for `png/jpg/webp`.
  - If not supported: implement WebP via a fallback path:
    - Prefer calling an external `cwebp` when present (detected via PATH).
    - Otherwise return a runtime error (exit 1) explaining how to install WebP tools and how to switch to png/jpg.
  - Document the decision and any external dependencies in the README (so users know what to expect).
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - The plan + README document the chosen backend and constraints (macOS version, external tool requirements if any).
  - The exit code is defined for “WebP requested but encoding unavailable” (runtime error, exit 1).
- **Validation**:
  - Manual: confirm a `.webp` output can be opened by Preview or validated by `file`.

### Task 1.2: Update README with screenshot contract + examples
- **Location**:
  - `crates/screen-record/README.md`
- **Description**: Extend the README to document screenshot mode end-to-end:
  - New flags (`--screenshot`, `--image-format`, and `--dir`) and their interactions with `--path`.
  - Updated "Mode rules" section to include screenshot mode as a mutually exclusive mode flag.
  - Updated "Output contract" section: stdout prints only the resolved output file path on success.
  - Naming rules: timestamp + resolved window identity; sanitization + truncation; collision suffix behavior.
  - At least 3 runnable screenshot examples (active window, app selection, explicit output path).
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - README flag table includes screenshot-related flags with correct defaults.
  - README clearly states when `--path` is required (recording) vs optional (screenshot).
  - README includes the exact default output directory and filename shape.
- **Validation**:
  - `rg -n \"--screenshot|image-format|screenshots\" crates/screen-record/README.md`

### Task 1.3: Add clap flags + enums for screenshot mode and image format
- **Location**:
  - `crates/screen-record/src/cli.rs`
- **Description**: Add new clap fields:
  - `--screenshot` (bool) to select screenshot mode.
  - `--image-format png|jpg|webp` (ValueEnum) to override extension-based inference.
  - `--dir path` (PathBuf) as an output directory used only when `--path` is omitted in screenshot mode.
  Keep existing `--format mov|mp4` for video container selection unchanged.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo run -p screen-record -- --help` shows the new flags.
  - No changes to existing flag names/semantics for recording modes.
- **Validation**:
  - `cargo run -p screen-record -- --help | rg \"--screenshot|--image-format|--dir\"`

### Task 1.4: Update mode detection and validation (add Screenshot mode)
- **Location**:
  - `crates/screen-record/src/run.rs`
- **Description**: Extend the mode model:
  - Add `Mode::Screenshot`.
  - Update `determine_mode` to treat `--screenshot` as a mutually exclusive mode flag alongside `--list-windows`, `--list-apps`, `--preflight`, `--request-permission`.
  - Implement `validate_screenshot_args`:
    - Requires exactly one selector (`--window-id`, `--active-window`, or `--app`).
    - Forbids recording-only flags (`--duration`, `--audio`, `--format`).
    - Forbids mixing `--dir` with `--path` (explicit file path wins; mixing is a usage error).
  - Forbid screenshot-only flags outside screenshot mode:
    - `--dir` and `--image-format` must exit 2 unless `--screenshot` is set (avoid silent ignore).
    - List/preflight/request-permission modes must reject screenshot flags the same way they reject recording flags.
  - Ensure error strings remain user-facing and consistent with existing usage errors (exit 2).
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Screenshot mode is reachable and validated, but (for now) may return a macOS-only "not implemented" runtime error.
  - Existing non-screenshot modes are unaffected.
  - Passing `--dir` or `--image-format` without `--screenshot` exits 2 with an actionable usage error.
- **Validation**:
  - `cargo test -p screen-record -- recording_test_mode` (ensures existing flows remain intact)

### Task 1.5: Update shell completions (Zsh + Bash)
- **Location**:
  - `completions/zsh/_screen-record`
  - `completions/bash/screen-record`
- **Description**: Add the new flags to completion scripts:
  - `--screenshot` as a mode flag.
  - `--image-format` value completion (`png jpg webp`).
  - `--dir` path completion (directory-like completion).
- **Dependencies**:
  - Task 1.3
- **Complexity**: 2
- **Acceptance criteria**:
  - Completion scripts contain the new flags and correct value sets.
- **Validation**:
  - `rg -n \"--screenshot|--image-format|--dir\" completions/zsh/_screen-record completions/bash/screen-record`

## Sprint 2: Path resolution + default naming + deterministic test mode for screenshots
**Goal**: Implement screenshot output path resolution, default naming rules, and test-mode fixture output so CI can fully validate screenshot mode without macOS APIs.
**Demo/Validation**:
- Command(s):
  - `cargo test -p screen-record -- screenshot` (new tests)
  - `AGENTS_SCREEN_RECORD_TEST_MODE=1 cargo test -p screen-record -- recording_test_mode`
- Verify:
  - Screenshot mode writes image fixtures and prints the resolved path to stdout.

### Task 2.1: Implement image format resolution and conflict checks
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/cli.rs`
- **Description**: Implement format inference for screenshot mode:
  - Resolve `ImageFormat` via (1) `--image-format`, else (2) `--path` extension, else default `png`.
  - Supported extensions: `png`, `jpg`, `jpeg`, `webp` (`jpeg` maps to `jpg` for output naming).
  - If `--image-format` conflicts with `--path` extension, exit 2 with a clear usage error.
  - If `--path` has an unsupported extension in screenshot mode, exit 2 (avoid silently producing mismatched files).
- **Dependencies**:
  - Task 1.3
- **Complexity**: 5
- **Acceptance criteria**:
  - Format is deterministic and validated in unit/integration tests.
  - Error messages follow the existing `--format ... conflicts with --path extension` style.
- **Validation**:
  - `cargo test -p screen-record -- format`

### Task 2.2: Define and test the default screenshot filename algorithm
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/types.rs`
  - `crates/screen-record/tests/` (new unit tests)
- **Description**: Add a pure filename generator that is testable and deterministic:
  - Input: pre-formatted timestamp string + resolved window identity (window id + owner name + window title) + image extension.
  - Output: a filename string only (no filesystem access).
  - Production: timestamp string is derived from local time and is safe for filenames (no `:` characters).
  - Test mode: allow overriding the timestamp string (e.g., fixed constant or `AGENTS_SCREEN_RECORD_TEST_TIMESTAMP`) so integration tests can assert exact filenames.
  Rules (example shape):
  - `screenshot-20260101-000000-win100-Terminal-Inbox.png`
  - Sanitize `owner` and `title`:
    - Replace whitespace runs with `-`.
    - Drop path separators and control characters.
    - Truncate by character boundary (never split UTF-8) to avoid invalid strings when titles include emoji/CJK.
    - If a sanitized segment becomes empty, omit it.
  - Collision handling (applies only to generated/default paths): if the target path already exists, append `-2`, `-3`, ... before the extension.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Unit tests cover sanitization, truncation, and collision suffix behavior.
  - Filename always includes the timestamp and the numeric window id (prefixed with `win`).
  - Unit tests include at least one non-ASCII window title case (emoji or CJK) to ensure truncation is UTF-8 safe.
- **Validation**:
  - `cargo test -p screen-record -- filename`

### Task 2.3: Implement screenshot output path resolution (`--path` vs `--dir` vs default)
- **Location**:
  - `crates/screen-record/src/run.rs`
- **Description**: Implement screenshot output path resolution:
  - If `--path` is provided:
    - If it has no extension, append the inferred extension (e.g. `.png`).
    - Resolve to an absolute path (relative paths are joined with `cwd`).
    - If the resolved path points at an existing directory, exit 2 with a usage error (explicit `--path` must be a file path).
    - If the output file already exists, overwrite it (match recording semantics); generated-name collision logic does not apply.
  - Else:
    - Use `--dir` if provided, else default to `./screenshots`.
    - If `--dir` exists and is not a directory, exit 2 with a usage error.
    - Join directory with the generated default filename.
    - If the generated path already exists, apply the `-2`, `-3`, ... collision suffix rule before writing.
  - Create output directories as needed.
  - Ensure stdout prints only the final resolved absolute path on success.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - `--screenshot` works without `--path` and creates `./screenshots` automatically.
  - `--dir` changes only the directory, not the filename algorithm.
  - Recording mode still requires `--path` and is unchanged.
  - `--path` pointing at a directory exits 2 with an actionable usage error.
  - `--dir` pointing at an existing non-directory path exits 2 with an actionable usage error.
- **Validation**:
  - `cargo test -p screen-record -- path`

### Task 2.4: Extend deterministic test mode to output screenshot fixtures
- **Location**:
  - `crates/screen-record/src/test_mode.rs`
  - `crates/screen-record/tests/recording_test_mode.rs`
  - `crates/screen-record/tests/fixtures/` (new fixtures)
- **Description**: Extend `AGENTS_SCREEN_RECORD_TEST_MODE=1` behavior:
  - Screenshot mode writes a fixture image matching the resolved image format.
  - When screenshot mode generates a default filename, use a deterministic timestamp string (constant or `AGENTS_SCREEN_RECORD_TEST_TIMESTAMP`) so integration tests can assert exact paths.
  - Add minimal fixtures:
    - `crates/screen-record/tests/fixtures/sample.png`
    - `crates/screen-record/tests/fixtures/sample.jpg`
    - `crates/screen-record/tests/fixtures/sample.webp`
  - Add/extend integration tests to assert:
    - stdout prints the resolved path
    - output file exists and is non-empty
    - default path behavior (when `--path` omitted) creates a deterministic file under `./screenshots/` (e.g. `./screenshots/screenshot-20260101-000000-win100-Terminal-Inbox.png`)
- **Dependencies**:
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests pass on non-macOS with `AGENTS_SCREEN_RECORD_TEST_MODE=1`.
- **Validation**:
  - `cargo test -p screen-record -- recording_test_mode`

## Sprint 3: macOS screenshot capture implementation (real capture + encoding)
**Goal**: Implement real screenshot capture on macOS 12+ using ScreenCaptureKit, producing correct `png`/`jpg`/`webp` outputs to the resolved path.
**Demo/Validation**:
- Command(s):
  - `cargo run -p screen-record -- --preflight`
  - `cargo run -p screen-record -- --screenshot --active-window --image-format png`
  - `cargo run -p screen-record -- --screenshot --active-window --image-format webp`
- Verify:
  - A valid image file is produced and opens in Preview.

### Task 3.1: Implement capture-first-frame pipeline via SCStream
- **Location**:
  - `crates/screen-record/src/macos/mod.rs`
  - `crates/screen-record/src/macos/screenshot.rs` (new)
  - `crates/screen-record/src/macos/stream.rs` (refactor helpers if needed)
- **Description**: Implement screenshot capture by starting an `SCStream` for the selected `SCWindow` and capturing the first screen `CMSampleBuffer`:
  - Reuse the same `SCContentFilter` + window dimension logic as recording.
  - Ensure all ScreenCaptureKit work runs on the main thread (mirror `record_window` main-thread guard).
  - Stop capture immediately after a valid frame is captured.
  - Add a bounded timeout (e.g., 2 seconds) that errors with a clear message if no frames arrive.
  - Map Objective-C `NSError` to `CliError::runtime` with actionable text.
- **Dependencies**:
  - none
- **Complexity**: 9
- **Acceptance criteria**:
  - On macOS with permission granted, capture returns exactly one frame and stops quickly.
  - Ctrl-C handling is not required for screenshot mode (single-shot), but must not regress recording behavior.
- **Validation**:
  - Manual: run screenshot command against a real window and observe it returns promptly.

### Task 3.2: Convert `CMSampleBuffer` to RGBA8 pixels
- **Location**:
  - `crates/screen-record/src/macos/screenshot.rs`
- **Description**: Implement a stable pixel conversion layer:
  - Extract `CVPixelBuffer` from the captured `CMSampleBuffer`.
  - Lock the base address and convert the incoming pixel format (typically BGRA) into an owned RGBA8 buffer.
  - Fail with a clear runtime error for unsupported pixel formats (include the format code in the message).
  - Keep the conversion code isolated so encoding backends can remain format-agnostic.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Conversion produces correct channel ordering (no red/blue swap) on at least one manual sanity test.
  - Unsupported formats fail fast with a readable runtime error (exit 1).
- **Validation**:
  - Manual: capture a screenshot and visually confirm colors are correct.

### Task 3.3: Encode and write PNG/JPG outputs (atomic write)
- **Location**:
  - `crates/screen-record/src/macos/screenshot.rs`
  - `crates/screen-record/build.rs` (add frameworks if needed, e.g. ImageIO)
  - `crates/screen-record/Cargo.toml` (add macOS-only deps if needed)
- **Description**: Implement image encoding for PNG and JPG and write output atomically:
  - Use the chosen system backend (preferred: ImageIO) to encode:
    - PNG: preserve alpha when available.
    - JPG: drop alpha and encode as RGB with a reasonable default quality (document the default).
  - Write to a temporary file in the same directory and rename to the final output path.
  - Ensure “explicit `--path` overwrite” behavior is implemented (remove existing target before rename when needed).
- **Dependencies**:
  - Task 3.2
- **Complexity**: 9
- **Acceptance criteria**:
  - Produced `.png` and `.jpg` files are valid and openable in Preview.
  - Writes are atomic (no partially-written target files on failure).
- **Validation**:
  - Manual:
    - `open ./screenshots/screenshot-20260101-000000-win100-Terminal-Inbox.png`
    - `open ./screenshots/screenshot-20260101-000000-win100-Terminal-Inbox.jpg`

### Task 3.4: Encode and write WebP outputs (system or external fallback)
- **Location**:
  - `crates/screen-record/src/macos/screenshot.rs`
- **Description**: Implement WebP encoding per the decision in Task 1.1:
  - If ImageIO can encode WebP: encode directly from RGBA8 pixels and write atomically like Task 3.3.
  - If ImageIO cannot encode WebP: implement an external fallback:
    - Require `cwebp` on PATH.
    - Encode a temporary PNG (lossless) to a temp path, then invoke `cwebp` to produce a temp `.webp`, then rename to the final output.
    - Ensure temporary files are cleaned up on success/failure.
  - If WebP is requested but no encoder is available, return a runtime error (exit 1) with actionable install guidance.
- **Dependencies**:
  - Task 1.1
  - Task 3.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Produced `.webp` files are valid and openable in Preview.
  - Missing WebP encoder yields a clear runtime error (exit 1) and does not create a broken output file.
- **Validation**:
  - Manual:
    - `open ./screenshots/screenshot-20260101-000000-win100-Terminal-Inbox.webp`

### Task 3.5: Integrate screenshot execution path into `run.rs`
- **Location**:
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/macos/mod.rs`
- **Description**: Wire the screenshot mode into the main runner:
  - Resolve shareable content, select the target window using existing selection.
  - Resolve output path + image format (Sprint 2 logic).
  - Execute test-mode fixture copy when enabled; otherwise call the macOS capture implementation.
  - Print only the resolved output path to stdout.
- **Dependencies**:
  - Task 2.4
  - Task 3.3
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Screenshot mode works in both test mode (fixture copy) and real macOS capture mode.
  - Existing recording flows remain unchanged.
- **Validation**:
  - `cargo test -p screen-record -- recording_test_mode`

## Sprint 4: Coverage, docs polish, and delivery checks
**Goal**: Make screenshot mode production-ready: full test coverage, updated documentation/completions, and all repo-required checks passing.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify:
  - fmt/clippy/test/zsh completion tests all pass.

### Task 4.1: Add integration tests for screenshot CLI behavior (test mode)
- **Location**:
  - `crates/screen-record/tests/recording_test_mode.rs`
  - `crates/screen-record/tests/cli_smoke.rs`
- **Description**: Add tests that assert:
  - `--screenshot` without `--path` creates `./screenshots` and outputs a `.png` file by default.
  - `--image-format` selects the correct fixture and file extension.
  - `--path` + `--image-format` conflicts error with exit 2.
  - Recording-only flags are rejected in screenshot mode (exit 2).
  - Screenshot-only flags are rejected outside screenshot mode (exit 2), e.g. `--dir` or `--image-format` without `--screenshot`.
  - Screenshot mode flags are rejected when combined with list/preflight/request-permission modes (exit 2).
  - Tests set a deterministic timestamp override (e.g., `AGENTS_SCREEN_RECORD_TEST_TIMESTAMP`) so default output paths can be asserted exactly.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests cover all screenshot formats and key validation errors.
- **Validation**:
  - `cargo test -p screen-record`

### Task 4.2: Update README examples and completions to match final behavior
- **Location**:
  - `crates/screen-record/README.md`
  - `completions/zsh/_screen-record`
  - `completions/bash/screen-record`
- **Description**: Ensure the docs and completions match the shipped behavior after implementation details settle:
  - Confirm default output dir + naming shown in examples.
  - Ensure flag descriptions reflect validation rules precisely.
- **Dependencies**:
  - Task 3.5
- **Complexity**: 2
- **Acceptance criteria**:
  - No drift between code behavior and README/completions.
- **Validation**:
  - Manual review + `rg` checks for flag presence.

### Task 4.3: Manual macOS validation checklist (permission + formats)
- **Location**:
  - `crates/screen-record/README.md` (optional: add a short "Manual validation" section for maintainers)
- **Description**: Run a manual verification pass on macOS:
  - `--preflight` and `--request-permission` behavior unchanged.
  - Screenshot capture works for each selector (`--window-id`, `--active-window`, `--app` + optional `--window-name`).
  - Verify all formats open in Preview.
- **Dependencies**:
  - Task 3.5
- **Complexity**: 3
- **Acceptance criteria**:
  - Maintainer can reproduce success with copy-pastable commands.
- **Validation**:
  - Manual.

## Testing Strategy
- Unit:
  - Filename sanitization/truncation logic.
  - Image format inference + conflict checks.
  - Path resolution behavior (`--path` vs `--dir` vs default).
- Integration:
  - `AGENTS_SCREEN_RECORD_TEST_MODE=1` end-to-end tests for screenshot creation, stdout contract, and validation errors.
  - Ensure existing recording test-mode tests remain unchanged and continue to pass.
- E2E/manual:
  - macOS: verify screenshot capture produces openable files for png/jpg/webp and exits quickly.
  - Permission denied scenarios: confirm errors are actionable.

## Risks & gotchas
- WebP encoding may not be supported uniformly via OS frameworks on macOS 12+; must validate and implement a clear fallback/error.
- ScreenCaptureKit requires main-thread execution; screenshot capture must enforce this like recording does.
- First frame timing: the earliest frame may be blank/black for some apps; consider waiting for a non-empty frame or a short grace period.
- Filename safety: window titles may contain slashes, emojis, or very long strings; sanitize and cap lengths to avoid filesystem issues.
- Pixel formats/color spaces: CVPixelBuffer may arrive in BGRA; conversions must be correct to avoid channel swapping.

## Rollback plan
- Screenshot mode is additive and gated by `--screenshot`; rollback is operationally simple:
  - Revert the screenshot-related commits and remove the new flags from completions/README.
  - If a partial rollback is needed, keep the CLI flags but return a clear runtime error (“screenshot temporarily disabled”) while retaining recording behavior.
