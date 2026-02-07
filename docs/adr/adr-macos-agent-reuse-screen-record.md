# ADR: Reuse Strategy for `macos-agent` with `screen-record`

- Status: Accepted
- Date: 2026-02-06
- Related plan: `docs/plans/macos-agent-ui-automation-cli-plan.md` (Sprint 1, Task 1.2)

## Context

`macos-agent` needs reliable macOS window discovery and screenshot capture quickly, while avoiding duplicated
unsafe ScreenCaptureKit integration code. `screen-record` already contains production code for:

- shareable content discovery (`crates/screen-record/src/macos/shareable.rs`)
- window screenshot capture (`crates/screen-record/src/macos/screenshot.rs`)
- selection logic (`crates/screen-record/src/select.rs`)
- common window/app/display models (`crates/screen-record/src/types.rs`)

Task 1.2 requires a concrete reuse decision for v1, including coupling risks, CI impact, and a migration path.

## Reuse Matrix

| Module / area | Current role | v1 decision | Why | Coupling risk and mitigation | CI implications |
| --- | --- | --- | --- | --- | --- |
| `types` | Canonical structs: `WindowInfo`, `DisplayInfo`, `AppInfo`, `ShareableContent` | **Keep + reuse directly** | Stable data model already used by discovery and selection; no unsafe code; lowest-risk reuse | Risk: field changes can ripple into `macos-agent`. Mitigation: consume through a thin adapter layer in `macos-agent` so output schema stays independent. | Cross-platform-safe data types; no additional platform linkage risk. |
| `select` | Deterministic selector resolution (`--window-id`, `--active-window`, `--app`, `--window-name`) and ambiguous candidate rendering | **Keep + reuse with adapter mapping** | Reusing avoids re-implementing ambiguity/frontmost behavior and keeps selector semantics consistent | Risk: tied to `CliError` and error text that mentions `screen-record` style flags. Mitigation: map `CliError` into `macos-agent` errors at adapter boundary; avoid exposing raw error text contract as public API. | Unit tests for selector behavior can run on all platforms (pure Rust). |
| `macos::shareable` | ScreenCaptureKit fetch + conversion into `ShareableContent` | **Keep + reuse directly (macOS-only path)** | Highest-value reuse: avoids duplicating fragile Objective-C callback/runloop integration | Risk: module path/API instability inside `screen-record` internals. Mitigation: isolate calls in one `macos-agent::targets` backend file and keep imports private. | Must gate usage with `#[cfg(target_os = "macos")]`; Linux CI must compile alternate stub path. |
| `macos::screenshot` | Window screenshot via ScreenCaptureKit stream + image encode/fallback | **Keep + reuse with format adapter** | Reuse avoids duplicating complex capture pipeline and encoding behavior | Risk: signature depends on `screen_record::cli::ImageFormat` (CLI-coupled type). Mitigation: keep a local `macos-agent` format enum and convert at one boundary function. | macOS-only runtime behavior; tests should use deterministic test-mode abstractions where possible. |
| Permission helpers (`macos::permissions`) | Screen Recording preflight/request + System Settings opener | **Do not reuse in `macos-agent` v1** | `macos-agent` preflight needs broader checks (Accessibility/Automation/cliclick/osascript), not just Screen Recording | Risk if reused: mixed responsibilities and side effects (`open` System Settings) that do not match `macos-agent` preflight UX. Mitigation: implement dedicated `macos-agent` preflight permission checks; optionally call lower-level APIs directly. | Keeps CI deterministic by avoiding side-effecting helper reuse in generic preflight tests. |

## Decision

For v1, `macos-agent` will **reuse** `screen-record` modules for `types`, `select`, `macos::shareable`, and
`macos::screenshot`, and will **not reuse** `screen-record` permission helpers.

No extraction spike is required in Sprint 1.

## Coupling Risks

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

## CI Implications

- `macos-agent` must keep all direct `screen_record::macos::*` calls inside `#[cfg(target_os = "macos")]`
  modules/functions so Linux workspace builds stay green.
- Selector logic reuse (`types`/`select`) remains testable in normal cross-platform unit tests.
- Screenshot/shareable integration tests should use deterministic seams (adapter trait + test doubles) for
  non-macOS CI, with macOS-only smoke tests for real capture paths.
- Do not rely on `cfg(coverage)` stubs in `screen-record` as the primary portability mechanism for
  `macos-agent`; treat them as coverage support for `screen-record` itself.

## Migration Path (If Extraction Becomes Necessary)

Extraction trigger conditions:

- `macos-agent` and another crate need the same capture API with repeated adapter glue.
- `screen-record` refactors cause repeated breakage in `macos-agent` imports/contracts.
- `CliError`/`ImageFormat` coupling becomes a blocker for independent command contracts.

Migration steps:

1. Add a stable facade in `screen-record` (e.g., `screen_record::capture_api`) that exposes
   capture-focused types and functions without CLI-domain enums/errors.
2. Move pure shared logic (`types`, selection algorithm core) behind that facade while keeping existing
   exports as compatibility wrappers.
3. Update `macos-agent` to consume only the facade.
4. After one release cycle, deprecate old direct module imports and remove compatibility wrappers.

## Actionable v1 Implementation (No Extra Spike)

1. In Task 2.1, build a `macos-agent` target adapter that reuses `screen_record::types` and
   `screen_record::select` behind local interfaces.
2. In Task 2.3, route screenshot observation through `screen_record::macos::shareable::fetch_shareable`
   + `screen_record::macos::screenshot::screenshot_window`, with local format/error conversion.
3. In Task 1.4, implement `macos-agent` preflight permission checks independently (including Accessibility
   and Automation checks), not by reusing `screen_record::macos::permissions`.

## Consequences

- Short-term delivery speed improves by reusing proven macOS capture code.
- `macos-agent` keeps command-contract control via adapter boundaries.
- A clear extraction route exists if cross-crate coupling grows.
