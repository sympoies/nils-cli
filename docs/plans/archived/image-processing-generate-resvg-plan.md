# Plan: image-processing generate subcommand with resvg/usvg/svg

> Status: superseded (historical archive)
>
> Replaced by: `docs/plans/image-processing-from-svg-llm-tooling-migration-plan.md`
> on 2026-02-12.

## Overview
This plan extends `image-processing` with a new `generate` subcommand for deterministic icon/simple-shape generation using Rust libraries (`svg`, `usvg`, `resvg`) while keeping existing ImageMagick-backed transform commands unchanged. The delivery focus is a low-risk additive feature: no behavioral regressions for existing subcommands and no replacement of current toolchain requirements for non-generate workflows. The `generate` path should avoid new runtime binary dependencies and work through pure Rust rendering for `png`/`webp`/`svg` outputs.

## Scope
- In scope:
  - Add `generate` as a new `image-processing` operation for app hint icons/simple shapes.
  - Implement rendering pipeline via `svg` document construction + `usvg` parsing + `resvg` rasterization.
  - Support deterministic output controls (format, size, foreground/background/stroke) and existing JSON/report contracts.
  - Keep current output-mode safety conventions (explicit output mode, overwrite rules, dry-run/report/json support).
  - Add comprehensive tests for CLI validation, rendering correctness basics, and non-regression of existing subcommands.
  - Update docs and diagnostics capability lists to expose the new subcommand.
- Out of scope:
  - Replacing ImageMagick for existing subcommands (`convert/resize/rotate/crop/pad/flip/flop/optimize/auto-orient/info`).
  - Advanced illustration features (text layout, gradients, filters, arbitrary imported SVG).
  - AI/image model generation or network-based generation workflows.

## Assumptions (if any)
1. The first iteration targets deterministic icon/simple-shape outputs and does not require feature parity with full vector editors.
2. Existing CLI behavior for non-`generate` operations remains contract-critical and must not change.
3. `generate` may run without ImageMagick present, but all existing transform subcommands continue to require current external binaries.
4. Mandatory repo gates in `DEVELOPMENT.md` still apply before delivery.

## CLI contract decisions for this plan
- New operation: `generate`.
- Input model: `generate` does not accept `--in`; generation parameters are provided by flags.
- Output model:
  - `--out` required for single artifact mode.
  - `--out-dir` allowed only when generating multiple variants in one invocation (explicitly defined by repeatable flags).
  - `--in-place` is invalid for `generate`.
- Initial icon/simple-shape contract:
  - Presets: `info`, `success`, `warning`, `error`, `help`.
  - Shape controls: `--size`, `--fg`, `--bg`, `--stroke`, `--stroke-width`, optional `--padding`.
  - Format controls: `--to png|webp|svg`.

## Sprint 1: Contract and architecture prep
**Goal**: Freeze CLI/output contract and isolate a generation architecture that does not disturb existing transform flow.
**Demo/Validation**:
- Command(s):
  - `cargo run -p image-processing -- --help`
  - `cargo run -p image-processing -- generate --help`
- Verify:
  - `generate` appears in help with explicit usage constraints.
  - Existing subcommands remain unchanged in help/flags.

**Parallelization notes**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.3` starts after 1.1 + 1.2.
- `Task 1.4` starts after 1.3 and locks baseline before Sprint 2.

### Task 1.1: Define `generate` CLI spec and validation matrix
- **Location**:
  - `crates/image-processing/README.md`
  - `crates/image-processing/src/cli.rs`
- **Description**: Define final flag contract for `generate` (presets, color/size/output/format rules), including forbidden/required flag combinations and explicit usage errors. Document deterministic defaults and naming conventions for generated outputs.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - `generate` contract is fully documented in crate README.
  - Required/forbidden flags are explicit and testable.
  - Output naming rules for `--out-dir` variant mode are documented.
- **Validation**:
  - `cargo run -p image-processing -- generate --help`
  - `sh -c "cargo run -p image-processing -- generate --in a.png --out out/x.png >/dev/null 2>&1; test $? -eq 2"`
  - `sh -c "cargo run -p image-processing -- generate --preset info --to png >/dev/null 2>&1; test $? -eq 2"`

### Task 1.2: Define backend split and toolchain gating rules
- **Location**:
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/toolchain.rs`
  - `crates/image-processing/src/processing.rs`
- **Description**: Design operation routing so `generate` can execute without ImageMagick detection, while non-generate paths keep current detection/fallback behavior. Specify where shared output/report plumbing remains common.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Operation-to-backend gating is documented in code comments and/or module docs.
  - No hidden dependency on ImageMagick remains in the `generate` execution path.
  - Existing backend detection semantics stay unchanged for transform subcommands.
- **Validation**:
  - `cargo build -p image-processing`
  - `sh -c "mkdir -p target/plan-generate-check && printf 'img' > target/plan-generate-check/a.png"`
  - `env PATH='' target/debug/image-processing generate --preset info --size 32 --fg '#ffffff' --bg '#0f62fe' --to svg --out target/plan-generate-check/info.svg --json`
  - `sh -c "env PATH='' target/debug/image-processing info --in target/plan-generate-check/a.png --json >/dev/null 2>&1; test $? -eq 1"`

### Task 1.3: Add rendering module scaffold and dependency wiring
- **Location**:
  - `crates/image-processing/Cargo.toml`
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/processing.rs`
  - `crates/image-processing/src/generate.rs`
- **Description**: Add `svg/usvg/resvg` dependencies, create a dedicated `generate` module API, and wire operation dispatch to keep generation logic isolated from ImageMagick command construction code.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - New module compiles and is invoked for `generate`.
  - Existing transform operations keep compiling with no feature loss.
  - Dependency additions are minimal and crate-scoped.
  - `generate` module API is fixed and documented so Sprint 2 tasks can run in parallel without interface churn.
- **Validation**:
  - `cargo check -p image-processing`
  - `cargo run -p image-processing -- generate --help`

### Task 1.4: Freeze legacy subcommand behavior baseline
- **Location**:
  - `crates/image-processing/tests/core_flows.rs`
  - `crates/image-processing/tests/edge_cases.rs`
  - `crates/image-processing/tests/dry_run_paths.rs`
- **Description**: Add/adjust explicit regression assertions for existing subcommands so that introducing `generate` cannot silently change legacy validation messages, exit codes, or command behavior.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - A dedicated regression section covers non-`generate` subcommands.
  - Baseline assertions include exit-code and key error-message stability checks.
  - Sprint 2 changes can fail fast on legacy regressions.
- **Validation**:
  - `cargo test -p image-processing --test core_flows`
  - `cargo test -p image-processing --test edge_cases`

## Sprint 2: Implement generation pipeline and CLI integration
**Goal**: Deliver functional `generate` output path with deterministic SVG-first rendering and parity-safe summary/report behavior.
**Demo/Validation**:
- Command(s):
  - `mkdir -p out/plan-generate-demo`
  - `cargo run -p image-processing -- generate --preset info --size 64 --fg '#ffffff' --bg '#0f62fe' --to png --out out/plan-generate-demo/info.png --json`
  - `cargo run -p image-processing -- generate --preset warning --size 64 --fg '#111111' --bg '#ffd166' --to svg --out out/plan-generate-demo/warning.svg --json`
- Verify:
  - Generated files are created with expected format and dimensions.
  - JSON summary includes `operation=generate`, backend marker, and output metadata.

**Parallelization notes**:
- `Task 2.1` and `Task 2.2` can run in parallel after `Task 1.3` API freeze.
- `Task 2.3` depends on 2.2.
- `Task 2.4` depends on 2.1 + 2.2 + 2.3.

### Task 2.1: Implement SVG document builders for preset icons/shapes
- **Location**:
  - `crates/image-processing/src/generate.rs`
  - `crates/image-processing/src/model.rs`
- **Description**: Implement deterministic SVG construction helpers for initial presets (`info/success/warning/error/help`) with shared geometry/color/stroke utilities and normalized viewBox sizing.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Each preset maps to one deterministic SVG document.
  - Invalid color/size/stroke inputs fail with actionable usage errors.
  - Common geometry logic is centralized (no preset-specific duplication for basic primitives).
- **Validation**:
  - `mkdir -p out/plan-task-2-1`
  - `cargo run -p image-processing -- generate --preset success --size 48 --fg '#ffffff' --bg '#2a9d8f' --to svg --out out/plan-task-2-1/success.svg --json`
  - `rg -n "<svg|viewBox" out/plan-task-2-1/success.svg`

### Task 2.2: Implement rasterization/export pipeline (`svg/usvg/resvg`)
- **Location**:
  - `crates/image-processing/src/generate.rs`
  - `crates/image-processing/src/model.rs`
- **Description**: Parse generated SVG via `usvg`, rasterize via `resvg` for `png/webp`, and preserve direct SVG output for `--to svg`, including explicit quality/alpha defaults for deterministic exports.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - `--to png|webp|svg` works with consistent dimensions and alpha behavior.
  - Export defaults for `webp` and alpha handling are explicit and documented in code.
  - Output bytes are deterministic for the same input options on the same platform/toolchain.
- **Validation**:
  - `mkdir -p out/plan-task-2-2`
  - `cargo run -p image-processing -- generate --preset warning --size 48 --fg '#111111' --bg '#ffd166' --to png --out out/plan-task-2-2/warning.png --json`
  - `cargo run -p image-processing -- generate --preset warning --size 48 --fg '#111111' --bg '#ffd166' --to webp --out out/plan-task-2-2/warning.webp --json`
  - `test -f out/plan-task-2-2/warning.png && test -f out/plan-task-2-2/warning.webp`

### Task 2.3: Integrate generate output-mode safety and file-write semantics
- **Location**:
  - `crates/image-processing/src/processing.rs`
  - `crates/image-processing/src/util.rs`
- **Description**: Integrate `generate` with existing safe-write behaviors (`--overwrite`, collision checks, dry-run, report/json artifact generation) without changing legacy subcommand semantics.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Dry-run emits planned actions and writes no output artifact.
  - Overwrite/collision behavior matches existing CLI conventions.
  - `--in-place` is rejected for `generate` with clear usage error.
- **Validation**:
  - `cargo test -p image-processing --test dry_run_paths`
  - `cargo test -p image-processing --test edge_cases`

### Task 2.4: Integrate `generate` validation and summary/report contracts
- **Location**:
  - `crates/image-processing/src/cli.rs`
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/processing.rs`
  - `crates/image-processing/src/report.rs`
- **Description**: Add subcommand-specific validation gates (`--in` invalid, `--in-place` invalid, required format/output flags), and ensure `summary.json`/`report.md` include coherent fields for generation runs.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Usage errors for invalid `generate` flag combinations return exit code 2 with clear messages.
  - Successful generation runs produce valid summary/report artifacts.
  - Existing subcommand validations remain unchanged.
- **Validation**:
  - `sh -c "cargo run -p image-processing -- generate --preset info --in a.png --to png --out out/x.png >/dev/null 2>&1; test $? -eq 2"`
  - `cargo test -p image-processing --test dry_run_paths`
  - `cargo test -p image-processing --test edge_cases`

## Sprint 3: Test hardening, docs, and integration visibility
**Goal**: Make `generate` production-ready with full regression coverage and updated operator-facing docs/capability surfaces.
**Demo/Validation**:
- Command(s):
  - `cargo test -p image-processing`
  - `cargo test -p agentctl --test diag_capabilities`
- Verify:
  - `generate` behavior is covered by deterministic tests.
  - Diagnostics and docs advertise the new capability accurately.

**Parallelization notes**:
- `Task 3.1` and `Task 3.2` can run in parallel.
- `Task 3.3` runs after 3.1 + 2.4.

### Task 3.1: Add deterministic integration tests for `generate`
- **Location**:
  - `crates/image-processing/tests/core_flows.rs`
  - `crates/image-processing/tests/edge_cases.rs`
  - `crates/image-processing/tests/dry_run_paths.rs`
  - `crates/image-processing/tests/common.rs`
- **Description**: Add test cases for successful generation (`png/webp/svg`), invalid flag combinations, output collision/overwrite handling, dry-run/report/json behavior, and backend independence from ImageMagick in `generate` mode.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 9
- **Acceptance criteria**:
  - Test suite covers happy path, error path, and edge path for `generate`.
  - Tests are deterministic and do not rely on host ImageMagick for `generate`.
  - Existing tests for legacy subcommands remain green.
- **Validation**:
  - `cargo test -p image-processing`

### Task 3.2: Update docs and usage examples
- **Location**:
  - `crates/image-processing/README.md`
  - `BINARY_DEPENDENCIES.md`
- **Description**: Document `generate` usage, examples, and safety constraints, including explicit note that ImageMagick remains required for transform subcommands but not for Rust-backed generation.
- **Dependencies**:
  - Task 1.1
  - Task 2.4
- **Complexity**: 5
- **Acceptance criteria**:
  - User-facing docs include at least three runnable `generate` examples.
  - Runtime dependency notes are explicit and non-contradictory.
- **Validation**:
  - `mkdir -p out/plan-doc-examples`
  - `cargo run -p image-processing -- generate --preset info --size 32 --fg '#ffffff' --bg '#0f62fe' --to png --out out/plan-doc-examples/info.png --json`
  - `cargo run -p image-processing -- generate --preset warning --size 32 --fg '#111111' --bg '#ffd166' --to webp --out out/plan-doc-examples/warning.webp --json`
  - `cargo run -p image-processing -- generate --preset help --size 32 --fg '#ffffff' --bg '#3a86ff' --to svg --out out/plan-doc-examples/help.svg --json`
  - `test -f out/plan-doc-examples/info.png && test -f out/plan-doc-examples/warning.webp && test -f out/plan-doc-examples/help.svg`

### Task 3.3: Update diagnostics capability inventory and run mandatory quality gate
- **Location**:
  - `crates/agentctl/src/diag/mod.rs`
- **Description**: Add `generate` to advertised `image-processing` capabilities in diagnostics, then run required repo quality checks and capture residual risks if any command fails.
- **Dependencies**:
  - Task 3.1
  - Task 2.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Diagnostics capability list includes `generate`.
  - Mandatory pre-delivery checks from project standards are executed and outcomes documented.
- **Validation**:
  - `cargo test -p agentctl --test diag_capabilities`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

## Testing Strategy
- Unit:
  - Preset geometry/color validation and SVG generation helper tests in `generate.rs`.
- Integration:
  - End-to-end CLI tests for `generate` success/error/dry-run/report/json paths in `crates/image-processing/tests/*.rs`.
- E2E/manual:
  - Manual spot checks by rendering representative presets at multiple sizes and confirming app-component legibility.
- Non-regression:
  - Existing `image-processing` integration suites remain mandatory and unchanged.

## Risks & gotchas
- `generate` has no input files, but current pipeline is input-centric; careless integration can break shared output-mode validations.
- `webp` export quality/alpha semantics can drift if rasterization and encoding assumptions are inconsistent.
- Backend detection currently happens early; if not refactored carefully, `generate` might still hard-fail on missing ImageMagick.
- SVG rendering determinism can be impacted by implicit defaults; explicit viewBox/size/stroke defaults are required.
- If external Codex skill docs are updated later, they should be tracked in a separate follow-up to avoid out-of-repo drift.

## Rollback plan
1. Keep `generate` code isolated in dedicated module and dispatch branch so rollback is a targeted revert.
2. If regressions occur, disable `generate` by removing `Operation::Generate` dispatch and CLI exposure while leaving existing commands intact.
3. Revert dependency additions in `crates/image-processing/Cargo.toml` and capability/docs updates in the same rollback PR.
4. Re-run `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh` plus `cargo test -p image-processing` to confirm legacy behavior restoration.
