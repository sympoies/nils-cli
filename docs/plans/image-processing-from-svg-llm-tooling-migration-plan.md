# Plan: image-processing remove generate and deliver from-svg + llm-svg tooling

## Overview
This plan replaces the current preset-only `generate` command with a source-SVG-first workflow that supports user-defined imagery. The implementation removes `generate` from CLI/runtime surfaces, introduces `--from-svg <path>` as the Rust-backed SVG input path for raster export, and adds tooling for an LLM-driven `intent -> SVG -> validated SVG -> PNG` pipeline. The delivery must also update all related repo policy/docs surfaces so the published contract no longer advertises `generate`.

## Scope
- In scope:
  - Remove `generate` command and all preset-specific runtime contracts from `image-processing`.
  - Add `--from-svg <path>` input contract for SVG-to-image conversion in `image-processing`.
  - Keep `usvg/resvg` Rust raster path for `png/webp` export from SVG source.
  - Add tooling required for LLM SVG generation workflow (prompt template, pipeline script, SVG validation/sanitization helper, repair loop support).
  - Update tests, diagnostics capability inventory, README/dependency docs, and plan/policy docs that currently reference `generate`.
  - Run mandatory project quality gates before delivery.
- Out of scope:
  - Building a hosted image model service or shipping a network provider inside `image-processing` itself.
  - Replacing existing non-SVG transform commands (`resize/rotate/crop/pad/flip/flop/optimize`) with Rust implementations.
  - Automatic style transfer or multi-object scene composition beyond single icon-scale SVG artifacts.

## Assumptions (if any)
1. `--from-svg` is introduced as an explicit source mode and is not combined with `--in`/`--recursive`/`--glob` in the same invocation.
2. LLM invocation is provider-agnostic via configurable command/environment; repo tooling will not hardcode one vendor endpoint.
3. `generate` removal is an intentional breaking change and will be documented in repo/user-facing docs.
4. Existing mandatory checks in `DEVELOPMENT.md` remain required before completion.

## Contract decisions for this plan
- `generate` subcommand is removed from user-facing CLI.
- New source flag: `--from-svg <path>`.
- `--from-svg` v1 contract:
  - Valid for conversion output flows only.
  - Requires explicit output mode via `--out` (single output) in v1.
  - Forbids `--in-place`, `--recursive`, `--glob`, and mixed use with `--in`.
  - Supports `--to png|webp|svg`.
- LLM workflow contract:
  - LLM output must be validated/sanitized before render.
  - Invalid SVG returns actionable diagnostics and optional repair prompt artifact.

## Sprint 1: Contract freeze and migration baseline
**Goal**: Lock the new CLI contract and migration policy before touching runtime internals.
**Demo/Validation**:
- Command(s):
  - `cargo run -p nils-image-processing -- --help`
  - `sh -c "mkdir -p target/plan-contract-check && cargo run -p nils-image-processing -- generate >/dev/null 2>target/plan-contract-check/generate-removed.err; test $? -eq 2; rg -qi 'unknown subcommand|unrecognized subcommand' target/plan-contract-check/generate-removed.err"`
- Verify:
  - Help no longer advertises `generate`.
  - New contract language for `--from-svg` is present and explicit.

**Parallelization notes**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.3` starts after 1.1 + 1.2.

### Task 1.1: Freeze CLI contract for removing `generate` and adding `--from-svg`
- **Location**:
  - `crates/image-processing/src/cli.rs`
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/README.md`
- **Description**: Define and document the new command contract that removes `generate` and introduces `--from-svg` as the SVG input path with explicit allowed/forbidden flag combinations.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - `Operation::Generate` and preset-only CLI help references are removed from contract docs.
  - `--from-svg` appears in CLI contract docs with required/forbidden matrix.
  - Breaking-change behavior is stated clearly for existing `generate` users.
- **Validation**:
  - `cargo run -p nils-image-processing -- --help`
  - `sh -c "mkdir -p target/plan-contract-check && cargo run -p nils-image-processing -- generate >/dev/null 2>target/plan-contract-check/generate-removed.err; test $? -eq 2; rg -qi 'unknown subcommand|unrecognized subcommand' target/plan-contract-check/generate-removed.err"`

### Task 1.2: Baseline migration + policy documentation updates for removed command
- **Location**:
  - `BINARY_DEPENDENCIES.md`
  - `docs/plans/archived/image-processing-generate-resvg-plan.md`
  - `docs/plans/image-processing-from-svg-llm-tooling-migration-plan.md`
- **Description**: Archive the old `generate` plan, update dependency/policy wording to `from-svg`/LLM workflow, and keep a clear migration trail for reviewers.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - No active policy/doc section claims `generate` is the current strategy.
  - Historical `generate` plan is moved under `docs/plans/archived/` with explicit superseded context.
  - New plan is the canonical implementation reference.
- **Validation**:
  - `rg -n "\bgenerate\b" BINARY_DEPENDENCIES.md crates/image-processing/README.md crates/image-processing/docs/runbooks/llm-svg-workflow.md`
  - `test -f docs/plans/archived/image-processing-generate-resvg-plan.md && test ! -f docs/plans/image-processing-generate-resvg-plan.md`
  - `rg -n "superseded|historical|replaced by --from-svg" docs/plans/archived/image-processing-generate-resvg-plan.md`

### Task 1.3: Add migration-focused regression tests for removed command behavior
- **Location**:
  - `crates/image-processing/tests/edge_cases.rs`
  - `crates/image-processing/tests/core_flows.rs`
- **Description**: Add explicit tests that `generate` is rejected with stable usage errors and that legacy non-SVG commands remain unaffected by contract changes.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests assert deterministic usage-error behavior for removed `generate` entrypoint.
  - Existing transform command regression tests remain green.
  - Migration behavior is covered in integration tests, not only docs.
- **Validation**:
  - `cargo test -p nils-image-processing --test edge_cases`
  - `cargo test -p nils-image-processing --test core_flows`

## Sprint 2: Runtime replacement with `--from-svg`
**Goal**: Remove `generate` runtime path and deliver production-safe `--from-svg` rendering.
**Demo/Validation**:
- Command(s):
  - `mkdir -p out/plan-from-svg`
  - `cargo run -p nils-image-processing -- convert --from-svg crates/image-processing/tests/fixtures/sample-icon.svg --to png --out out/plan-from-svg/sample.png --json`
  - `cargo run -p nils-image-processing -- convert --from-svg crates/image-processing/tests/fixtures/sample-icon.svg --to webp --out out/plan-from-svg/sample.webp --json`
- Verify:
  - Rendering works without ImageMagick for `--from-svg` path.
  - Summary JSON records SVG source mode and output metadata.

**Parallelization notes**:
- `Task 2.1` starts first and unblocks all later tasks.
- `Task 2.2` and `Task 2.3` can run in parallel after 2.1.
- `Task 2.4` depends on 2.2 + 2.3.

### Task 2.1: Remove `generate` runtime/module surfaces
- **Location**:
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/processing.rs`
  - `crates/image-processing/src/generate.rs`
  - `crates/image-processing/src/cli.rs`
- **Description**: Remove dispatch and processing branches dedicated to `generate` (including preset model), and refactor shared flow so no dead branches remain.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - `generate` module/dispatch code is removed or replaced with non-preset abstractions.
  - Build succeeds without runtime references to removed `generate` symbols.
  - Error/help output no longer suggests preset generation mode.
- **Validation**:
  - `cargo check -p nils-image-processing`
  - `sh -c "mkdir -p target/plan-contract-check && cargo run -p nils-image-processing -- generate >/dev/null 2>target/plan-contract-check/generate-removed.err; test $? -eq 2; rg -qi 'unknown subcommand|unrecognized subcommand' target/plan-contract-check/generate-removed.err"`

### Task 2.2: Implement `--from-svg` parse/sanitize/render pipeline
- **Location**:
  - `crates/image-processing/src/processing.rs`
  - `crates/image-processing/src/model.rs`
  - `crates/image-processing/src/util.rs`
- **Description**: Add SVG input loading and Rust-native render/export path (`usvg/resvg` + encoders) driven by `--from-svg` contract, with deterministic defaults.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - `--from-svg` supports `png|webp|svg` output.
  - Invalid/malformed SVG returns actionable usage/runtime errors.
  - Output metadata (format, size, alpha/channels when applicable) is filled consistently.
- **Validation**:
  - `cargo run -p nils-image-processing -- convert --from-svg crates/image-processing/tests/fixtures/sample-icon.svg --to png --out out/plan-from-svg/sample.png --json`
  - `cargo run -p nils-image-processing -- convert --from-svg crates/image-processing/tests/fixtures/sample-icon.svg --to webp --out out/plan-from-svg/sample.webp --json`
  - `test -f out/plan-from-svg/sample.png && test -f out/plan-from-svg/sample.webp`

### Task 2.3: Integrate toolchain/output-mode gating for SVG source mode
- **Location**:
  - `crates/image-processing/src/toolchain.rs`
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/processing.rs`
- **Description**: Ensure `--from-svg` path bypasses ImageMagick detection while preserving legacy behavior for non-SVG source commands; enforce output safety conventions.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - `PATH=''` still fails for legacy ImageMagick-dependent flows.
  - `PATH=''` succeeds for `--from-svg` render flow.
  - `--from-svg` forbids incompatible flags (`--in`, `--recursive`, `--glob`, `--in-place`) with exit code 2.
- **Validation**:
  - `cargo build -p nils-image-processing`
  - `env PATH='' target/debug/image-processing convert --from-svg crates/image-processing/tests/fixtures/sample-icon.svg --to png --out out/plan-from-svg/path-empty.png --json`
  - `sh -c "env PATH='' target/debug/image-processing info --in crates/image-processing/tests/fixtures/sample-raster.png --json >/dev/null 2>&1; test $? -eq 1"`

### Task 2.4: Update summary/report contract for SVG-source runs
- **Location**:
  - `crates/image-processing/src/model.rs`
  - `crates/image-processing/src/report.rs`
  - `crates/image-processing/src/processing.rs`
- **Description**: Add source-mode fields (including `from_svg` provenance) to summary/report outputs so downstream tooling can audit generated artifacts.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - JSON summary includes explicit indication that source is SVG path mode.
  - Markdown report includes source SVG path and produced output path(s).
  - Dry-run/report combinations behave consistently with existing conventions.
- **Validation**:
  - `cargo test -p nils-image-processing --test dry_run_paths`
  - `cargo test -p nils-image-processing --test core_flows`

## Sprint 3: LLM -> SVG tooling chain
**Goal**: Provide practical, provider-agnostic tooling so an agent can turn user intent into valid SVG artifacts before raster export.
**Demo/Validation**:
- Command(s):
  - `SVG_LLM_CMD='cat crates/image-processing/tests/fixtures/llm-svg-valid.svg' scripts/image-processing/llm_svg_pipeline.sh --intent "traffic car icon" --out-svg out/plan-llm/traffic-car.svg --dry-run`
  - `cargo run -p nils-image-processing -- svg-validate --in out/plan-llm/traffic-car.svg --out out/plan-llm/traffic-car.cleaned.svg`
  - `cargo run -p nils-image-processing -- convert --from-svg out/plan-llm/traffic-car.cleaned.svg --to png --out out/plan-llm/traffic-car.png --json`
- Verify:
  - Pipeline emits prompt + SVG candidate/repair artifacts deterministically.
  - Sanitized SVG is renderable through `--from-svg`.

**Parallelization notes**:
- `Task 3.1` and `Task 3.2` can run in parallel.
- `Task 3.3` depends on 3.1 + 3.2.
- `Task 3.4` depends on 3.3.

### Task 3.1: Add LLM prompt contract assets for icon SVG generation
- **Location**:
  - `crates/image-processing/assets/llm-svg-system-prompt.md`
  - `crates/image-processing/assets/llm-svg-output-contract.md`
  - `crates/image-processing/docs/runbooks/llm-svg-workflow.md`
- **Description**: Define strict prompt/output contract (single SVG document, allowed tags/attrs, icon framing rules, deterministic size/viewBox constraints) for agent-driven generation.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Prompt assets define unambiguous required SVG shape/constraints.
  - Contract includes failure and repair instructions for invalid output.
  - Runbook documents expected pipeline inputs/outputs.
- **Validation**:
  - `rg -n "viewBox|allowed tags|single <svg" crates/image-processing/assets/llm-svg-system-prompt.md crates/image-processing/assets/llm-svg-output-contract.md`

### Task 3.2: Implement SVG validation/sanitization command for LLM outputs
- **Location**:
  - `crates/image-processing/src/cli.rs`
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/svg_validate.rs`
  - `crates/image-processing/tests/edge_cases.rs`
- **Description**: Add CLI entrypoint for validating and sanitizing candidate SVG files, returning machine-readable diagnostics for downstream repair loops.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Command rejects malformed or policy-violating SVG with actionable error categories.
  - Sanitized SVG output is deterministic for identical input.
  - Validation command has integration coverage for pass/fail cases.
- **Validation**:
  - `cargo run -p nils-image-processing -- svg-validate --in crates/image-processing/tests/fixtures/llm-svg-valid.svg --out out/plan-llm/valid.cleaned.svg`
  - `sh -c "cargo run -p nils-image-processing -- svg-validate --in crates/image-processing/tests/fixtures/llm-svg-invalid.svg --out out/plan-llm/invalid.cleaned.svg >/dev/null 2>&1; test $? -ne 0"`

### Task 3.3: Add provider-agnostic LLM orchestration script with repair loop hooks
- **Location**:
  - `scripts/image-processing/llm_svg_pipeline.sh`
  - `scripts/image-processing/llm_svg_repair_prompt.sh`
  - `tests/zsh/image-processing-llm-svg.test.zsh`
- **Description**: Provide a scriptable pipeline that builds prompts, invokes configurable LLM command (`SVG_LLM_CMD`), extracts SVG, runs validation, and emits repair prompts/artifacts when needed.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 9
- **Acceptance criteria**:
  - Script runs in dry-run mode without network and emits expected prompt/artifact files.
  - Non-dry-run mode supports command injection via `SVG_LLM_CMD` and handles stderr/exit codes safely.
  - Repair prompt artifact is emitted when validation fails.
- **Validation**:
  - `zsh -f tests/zsh/image-processing-llm-svg.test.zsh`
  - `SVG_LLM_CMD='cat crates/image-processing/tests/fixtures/llm-svg-valid.svg' scripts/image-processing/llm_svg_pipeline.sh --intent "sun icon" --out-svg out/plan-llm/sun.svg --dry-run`
  - `test -f out/plan-llm/sun.svg`
  - `test -f out/plan-llm/sun.prompt.md`

### Task 3.4: Add end-to-end fixture path for intent -> SVG -> PNG workflow
- **Location**:
  - `crates/image-processing/tests/core_flows.rs`
  - `crates/image-processing/tests/fixtures/llm-svg-valid.svg`
  - `crates/image-processing/docs/runbooks/llm-svg-workflow.md`
- **Description**: Add deterministic E2E fixture tests and runbook examples that prove generated/sanitized SVG can be rendered by `--from-svg` into expected PNG/WebP outputs.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 7
- **Acceptance criteria**:
  - E2E tests cover success and repair-needed branches.
  - Runbook includes commands reproducible in local dev environment.
  - Artifact paths and contracts are consistent across docs/tests/scripts.
- **Validation**:
  - `cargo test -p nils-image-processing --test core_flows`
  - `cargo run -p nils-image-processing -- convert --from-svg crates/image-processing/tests/fixtures/llm-svg-valid.svg --to png --out out/plan-llm/fixture.png --json`

## Sprint 4: Documentation/diagnostics cleanup and release-quality gates
**Goal**: Ensure repository-facing contracts are internally consistent and fully validated after the breaking change.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-agentctl --test diag_capabilities`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
- Verify:
  - Diagnostics no longer claim removed `generate` capability.
  - Required repository gates pass with updated docs/tests.

**Parallelization notes**:
- `Task 4.1` and `Task 4.2` can run in parallel.
- `Task 4.3` runs after 4.1 + 4.2 + Sprint 3 completion.

### Task 4.1: Update user-facing/spec docs for new canonical workflow
- **Location**:
  - `crates/image-processing/README.md`
  - `BINARY_DEPENDENCIES.md`
  - `crates/image-processing/docs/runbooks/llm-svg-workflow.md`
- **Description**: Replace all active `generate` guidance with `--from-svg` + LLM tooling workflow examples, including migration notes and safety requirements.
- **Dependencies**:
  - Task 2.4
  - Task 3.4
- **Complexity**: 6
- **Acceptance criteria**:
  - Docs do not present `generate` as supported behavior.
  - README includes runnable `--from-svg` + pipeline examples.
  - Dependency notes clearly describe ImageMagick vs Rust path behavior.
- **Validation**:
  - `rg -n "\bgenerate\b" crates/image-processing/README.md BINARY_DEPENDENCIES.md crates/image-processing/docs/runbooks/llm-svg-workflow.md`
  - `cargo run -p nils-image-processing -- --help`

### Task 4.2: Update diagnostics capability inventory and tests
- **Location**:
  - `crates/agentctl/src/diag/mod.rs`
  - `crates/agentctl/tests/diag_capabilities.rs`
- **Description**: Remove stale `generate` capability and add capability descriptors for supported SVG-source/validation workflow surfaces.
- **Dependencies**:
  - Task 2.1
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Diagnostics output reflects actual command surface after migration.
  - Capability tests pass and assert new expected set.
  - No stale references to removed command remain in diag outputs.
- **Validation**:
  - `cargo test -p nils-agentctl --test diag_capabilities`

### Task 4.3: Run mandatory quality gates and migration smoke checks
- **Location**:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `scripts/ci/coverage-summary.sh`
  - `crates/image-processing/tests`
- **Description**: Execute required repo checks, coverage gate, and explicit migration smoke commands for removed `generate` + new `--from-svg` path.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Required checks from `DEVELOPMENT.md` complete successfully.
  - Coverage remains >= 85.00%.
  - Smoke checks confirm `generate` removal and `--from-svg` success path.
- **Validation**:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `mkdir -p target/coverage`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - `sh -c "mkdir -p target/plan-contract-check && cargo run -p nils-image-processing -- generate >/dev/null 2>target/plan-contract-check/generate-removed.err; test $? -eq 2; rg -qi 'unknown subcommand|unrecognized subcommand' target/plan-contract-check/generate-removed.err"`
  - `cargo run -p nils-image-processing -- convert --from-svg crates/image-processing/tests/fixtures/sample-icon.svg --to png --out out/plan-from-svg/final-smoke.png --json`

## Testing Strategy
- Unit:
  - SVG parse/sanitize/contract tests for new validation module and source-mode parsing.
- Integration:
  - `image-processing` CLI tests for removed `generate`, `--from-svg` success/error modes, and report/json contract updates.
- E2E/manual:
  - LLM pipeline dry-run and fixture-driven intent-to-png end-to-end flow.
- Non-regression:
  - Existing non-SVG transform commands remain mandatory in test suite.

## Risks & gotchas
- Removing `generate` is a breaking contract and can silently break downstream workflows if migration messaging is weak.
- LLM-produced SVG may include unsupported tags/attributes, malformed XML, or unsafe constructs; sanitization gate is mandatory.
- `--from-svg` toolchain gating must not accidentally relax ImageMagick requirements for unrelated legacy commands.
- Provider-agnostic LLM script design can become brittle if stdout extraction and error signaling are not strongly specified.
- Superseded docs/plans may cause confusion if not explicitly marked as historical.

## Rollback plan
1. Reintroduce prior `generate` command surface by reverting CLI/dispatch removal commits.
2. Disable `--from-svg` and new svg-validation/LLM tooling entrypoints behind a targeted rollback commit if instability appears.
3. Restore diagnostics capability list and docs to pre-migration state in the same rollback PR.
4. Re-run `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh` and coverage gate to confirm legacy stability.
