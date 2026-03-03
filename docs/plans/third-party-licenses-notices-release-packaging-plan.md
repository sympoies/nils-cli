# Plan: Third-Party Licenses/Notices Automation and Release Packaging

## Overview

This plan introduces a deterministic automation flow that generates `THIRD_PARTY_LICENSES.md` and `THIRD_PARTY_NOTICES.md`, enforces
freshness in CI, and ensures both files are included in GitHub release tarballs. The rollout is sprint-gated: generator contract first,
then CI enforcement, then release packaging updates. Existing CLI behavior and runtime contracts remain unchanged outside legal/compliance
artifacts. The implementation prioritizes deterministic output, fail-closed CI behavior, and release artifact traceability.

## Scope

- In scope:
  - Add a repository script to generate `THIRD_PARTY_LICENSES.md` and `THIRD_PARTY_NOTICES.md` from `Cargo.lock`/`cargo metadata`.
  - Define and document the artifact contract (sections, deterministic ordering, failure behavior, regeneration workflow).
  - Add a CI audit command that fails when generated artifacts drift from source-of-truth dependencies.
  - Wire the audit into required checks and GitHub CI workflows.
  - Update release packaging to include both files in tarballs.
- Out of scope:
  - Legal review of every upstream dependency notice requirement beyond machine-extractable metadata and obvious notice files.
  - Replacing the existing release model or changing binary build targets.
  - Introducing new external package managers or non-repo build systems.

## Assumptions

1. `cargo metadata --format-version 1 --locked` remains the canonical dependency inventory source.
2. `python3` and `cargo` are available in local dev and CI (already part of repository tooling assumptions).
3. Root-level `THIRD_PARTY_LICENSES.md` and `THIRD_PARTY_NOTICES.md` are acceptable release artifact locations.
4. Some crates may not ship an explicit `NOTICE` file; the notices artifact may include deterministic "no explicit NOTICE file discovered"
   markers for those entries.

## Success criteria

- Running a single generator command updates both third-party artifacts deterministically with no manual editing.
- CI fails when either artifact is stale relative to the current dependency graph.
- `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh` includes the new audit gate.
- Release tarballs produced by `.github/workflows/release.yml` include:
  - `THIRD_PARTY_LICENSES.md`
  - `THIRD_PARTY_NOTICES.md`
- README release packaging contract documents both files as required shipped assets.

## Sprint 1: Generator contract and deterministic artifacts

**Goal**: Implement deterministic generation for licenses + notices and lock the artifact contract before CI/release integration. **Demo/Validation**:

- Command(s):
  - `plan-tooling validate --file docs/plans/third-party-licenses-notices-release-packaging-plan.md`
  - `bash scripts/generate-third-party-artifacts.sh --check`
  - `bash scripts/generate-third-party-artifacts.sh --write`
- Verify:
  - Both root artifacts are generated from locked dependency metadata.
  - Output ordering/content are deterministic and re-runs are no-op when dependencies are unchanged.
- **PR grouping intent**: group
- **Execution Profile**: parallel-x2
- Sprint scorecard:
  - `Execution Profile`: parallel-x2
  - `TotalComplexity`: 16
  - `CriticalPathComplexity`: 12
  - `MaxBatchWidth`: 2
  - `OverlapHotspots`: `scripts/generate-third-party-artifacts.sh`; `THIRD_PARTY_LICENSES.md`; `THIRD_PARTY_NOTICES.md`

### Task 1.1: Define artifact schema and generation contract

- **Location**:
  - docs/specs/third-party-artifacts-contract-v1.md
  - THIRD_PARTY_LICENSES.md
  - THIRD_PARTY_NOTICES.md
- **Description**: Define required sections, column schema, deterministic ordering rules, and failure semantics for both generated files.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Contract spec defines mandatory headers/sections for both artifacts.
  - Contract includes deterministic ordering keys and regeneration triggers.
  - Contract documents `--check` vs `--write` behavior.
- **Validation**:
  - `test -f docs/specs/third-party-artifacts-contract-v1.md`
  - `rg -n 'THIRD_PARTY_LICENSES\.md|THIRD_PARTY_NOTICES\.md|deterministic|--check|--write' docs/specs/third-party-artifacts-contract-v1.md`

### Task 1.2: Implement generator entrypoint for both artifacts

- **Location**:
  - scripts/generate-third-party-artifacts.sh
  - THIRD_PARTY_LICENSES.md
  - THIRD_PARTY_NOTICES.md
- **Description**: Build a single entrypoint that reads `cargo metadata --locked`, renders license summary/list plus notice sections, and writes
  both artifacts with stable sorting and formatting.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `--write` regenerates both artifacts in one run.
  - `--check` exits non-zero if in-repo artifacts differ from regenerated output.
  - Generator output is stable across repeated runs with unchanged lockfile.
- **Validation**:
  - `bash scripts/generate-third-party-artifacts.sh --write`
  - `bash scripts/generate-third-party-artifacts.sh --check`
  - `git diff --exit-code -- THIRD_PARTY_LICENSES.md THIRD_PARTY_NOTICES.md`

### Task 1.3: Add deterministic notice extraction policy and fallback markers

- **Location**:
  - scripts/generate-third-party-artifacts.sh
  - docs/specs/third-party-artifacts-contract-v1.md
  - THIRD_PARTY_NOTICES.md
- **Description**: Implement deterministic notice extraction (crate notice/license-file references when discoverable) and explicit
  fallback text when no crate-level notice file is found.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `THIRD_PARTY_NOTICES.md` contains per-crate notice entries with deterministic ordering.
  - Entries without discoverable notice files use standardized fallback wording.
  - Contract spec defines fallback wording exactly.
- **Validation**:
  - `bash scripts/generate-third-party-artifacts.sh --write`
  - `rg -n '## Dependency Notices|No explicit NOTICE file discovered' THIRD_PARTY_NOTICES.md`
  - `rg -n 'fallback wording' docs/specs/third-party-artifacts-contract-v1.md`

### Task 1.4: Add script-level regression tests and usage docs

- **Location**:
  - tests/third-party-artifacts/generator.test.sh
  - README.md
  - BINARY_DEPENDENCIES.md
- **Description**: Add regression tests for `--check/--write` behavior and document the generator as a repository-local compliance tool.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Test covers no-op determinism and drift detection paths.
  - README includes regeneration command for contributors.
  - `BINARY_DEPENDENCIES.md` references the new script entrypoint.
- **Validation**:
  - `bash tests/third-party-artifacts/generator.test.sh`
  - `rg -n 'generate-third-party-artifacts\.sh' README.md BINARY_DEPENDENCIES.md`

## Sprint 2: CI freshness gate and required-check integration

**Goal**: Enforce generated-artifact freshness in local required checks and GitHub CI for both Linux and macOS jobs. **Demo/Validation**:

- Command(s):
  - `bash scripts/ci/third-party-artifacts-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify:
  - CI/local checks fail closed when artifacts are stale.
  - The new audit gate is part of the canonical required-checks entrypoint.
- **PR grouping intent**: group
- **Execution Profile**: parallel-x2
- Sprint scorecard:
  - `Execution Profile`: parallel-x2
  - `TotalComplexity`: 15
  - `CriticalPathComplexity`: 11
  - `MaxBatchWidth`: 2
  - `OverlapHotspots`: `scripts/ci/third-party-artifacts-audit.sh`; `.github/workflows/ci.yml`; `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

### Task 2.1: Implement strict third-party artifact audit script

- **Location**:
  - scripts/ci/third-party-artifacts-audit.sh
- **Description**: Add a strict CI audit script that regenerates artifacts in check mode and reports precise drift diagnostics.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Supports `--strict` and emits PASS/WARN/FAIL style messages consistent with existing audits.
  - Fails when generated output differs or required files are missing.
- **Validation**:
  - `bash scripts/ci/third-party-artifacts-audit.sh --strict`
  - `bash scripts/ci/third-party-artifacts-audit.sh`

### Task 2.2: Add audit gate to required-checks entrypoint and contributor docs

- **Location**:
  - .agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
  - DEVELOPMENT.md
  - AGENTS.md
- **Description**: Insert the new audit into full-check flows and update required-check documentation to keep local/CI policy aligned.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Required-check script runs third-party audit before fmt/clippy/tests.
  - `DEVELOPMENT.md` and root `AGENTS.md` list the new required command.
- **Validation**:
  - `rg -n 'third-party-artifacts-audit\.sh' .agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `rg -n 'third-party-artifacts-audit\.sh' DEVELOPMENT.md AGENTS.md`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

### Task 2.3: Wire audit into GitHub CI workflows (Linux + macOS)

- **Location**:
  - .github/workflows/ci.yml
- **Description**: Add explicit third-party artifact audit steps in Linux and macOS CI jobs, matching shell conventions used by each job.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 4
- **Acceptance criteria**:
  - CI test and test_macos jobs each execute the new audit command.
  - Failure surface is visible before expensive build/test stages when feasible.
- **Validation**:
  - `rg -n 'third-party-artifacts-audit\.sh' .github/workflows/ci.yml`
  - `test "$(rg -n 'third-party-artifacts-audit\\.sh' .github/workflows/ci.yml | wc -l | tr -d ' ')" -ge 2`

### Task 2.4: Add CI regression tests for audit script behavior

- **Location**:
  - tests/third-party-artifacts/audit.test.sh
  - scripts/ci/third-party-artifacts-audit.sh
- **Description**: Add shell regression tests that assert strict/non-strict behavior and drift detection output semantics.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Tests assert exit code behavior for clean and dirty generated artifacts.
  - Tests validate core PASS/FAIL message format.
- **Validation**:
  - `bash tests/third-party-artifacts/audit.test.sh`

## Sprint 3: Release workflow packaging and artifact verification

**Goal**: Ensure release tarballs always include regenerated third-party artifacts and fail if packaging contract is broken. **Demo/Validation**:

- Command(s):
  - `bash scripts/generate-third-party-artifacts.sh --check`
  - `host_target="$(rustc -vV | sed -n 's/^host: //p')" && cargo build --release --workspace --locked --target "$host_target"`
  - `host_target="$(rustc -vV | sed -n 's/^host: //p')" && bash scripts/ci/release-tarball-third-party-audit.sh --target "$host_target"`
- Verify:
  - Release packaging regenerates and ships both artifacts.
  - Tarball contract fails closed when either file is missing.
- **PR grouping intent**: per-sprint
- **Execution Profile**: serial
- Sprint scorecard:
  - `Execution Profile`: serial
  - `TotalComplexity`: 14
  - `CriticalPathComplexity`: 14
  - `MaxBatchWidth`: 1
  - `OverlapHotspots`: `.github/workflows/release.yml`; `README.md`; `scripts/ci/release-tarball-third-party-audit.sh`

### Task 3.1: Regenerate third-party artifacts in release build job

- **Location**:
  - .github/workflows/release.yml
  - scripts/generate-third-party-artifacts.sh
- **Description**: Add a release workflow step that runs generator check/write deterministically before packaging and fails on generation errors.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
  - Task 2.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Each matrix build job runs the generator before tarball creation.
  - Release job halts if generation fails.
- **Validation**:
  - `rg -n 'generate-third-party-artifacts\.sh' .github/workflows/release.yml`

### Task 3.2: Include artifacts in tarball packaging contract

- **Location**:
  - .github/workflows/release.yml
  - README.md
- **Description**: Update packaging copy steps and README contract text so release tarballs explicitly contain both third-party artifacts.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Package step copies `THIRD_PARTY_LICENSES.md` and `THIRD_PARTY_NOTICES.md` into release output directory.
  - README release artifact contract lists both files.
- **Validation**:
  - `rg -n 'THIRD_PARTY_LICENSES\.md|THIRD_PARTY_NOTICES\.md' .github/workflows/release.yml README.md`
  - `tar -tzf dist/nils-cli-*.tar.gz | rg 'THIRD_PARTY_(LICENSES|NOTICES)\\.md'`

### Task 3.3: Add release tarball compliance audit script

- **Location**:
  - scripts/ci/release-tarball-third-party-audit.sh
  - .github/workflows/release.yml
- **Description**: Add a script that inspects generated tarball contents and fails if required third-party artifacts are absent.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Audit validates tarball file list includes both third-party artifacts.
  - Release workflow executes the audit after tarball creation and before upload.
- **Validation**:
  - `bash scripts/ci/release-tarball-third-party-audit.sh --target x86_64-unknown-linux-gnu`
  - `rg -n 'release-tarball-third-party-audit\.sh' .github/workflows/release.yml`

### Task 3.4: Add end-to-end release fixture test for artifact presence

- **Location**:
  - scripts/ci/release-tarball-third-party-audit.sh
- **Description**: Add an E2E shell test that builds a local release package fixture and asserts third-party files exist in extracted archive.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Test verifies tarball contains both root third-party artifacts and reports clear failure diagnostics.
  - Test is runnable in CI-like environments without interactive dependencies.
- **Validation**:
  - `bash scripts/ci/release-tarball-third-party-audit.sh --help`

## Testing Strategy

- Unit:
  - Script-level parsing/render helpers (if split into helper functions) for deterministic sorting and fallback marker rendering.
- Integration:
  - Generator `--write`/`--check` regression tests and CI audit strict/non-strict mode tests.
- E2E/manual:
  - Release package fixture test and workflow-level verification that uploaded tarballs include both third-party artifacts.

## Risks & gotchas

- Notice completeness risk: some crates may have nuanced notice obligations not discoverable via metadata alone.
- Determinism risk: timestamps or environment-dependent paths can cause false CI drift unless explicitly normalized.
- CI runtime risk: generating notice content from vendored crate sources may increase job time; audit should remain lightweight.
- Release race risk: if generator runs after packaging copy operations, tarballs can ship stale artifacts.

## Rollback plan

- Revert CI/release workflow wiring first (`.github/workflows/ci.yml`, `.github/workflows/release.yml`) to restore previous pipeline behavior.
- Keep generator script and docs behind non-blocking manual invocation until drift/issues are resolved.
- If release packaging gate causes urgent blocker, temporarily disable only `release-tarball-third-party-audit.sh` step while retaining
  `scripts/generate-third-party-artifacts.sh --check` in CI to preserve freshness signal.
- Re-run required checks and one release dry-run after rollback adjustments to confirm pipeline stability.
