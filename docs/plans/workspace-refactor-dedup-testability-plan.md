# Plan: Workspace Refactor for Deduplication, Testability, and Maintainability

## Overview

This plan refactors the workspace in four sprint-gated areas: CI, production crates, test/shared crates, and documentation. The intent is
to remove obsolete code and docs outright, stop carrying compatibility-only paths, extract genuinely reusable helpers, and make the codebase
easier to test and maintain. The rollout stays sequential across sprints, but uses controlled parallelism inside sprints where file overlap
is manageable.

## Scope

- In scope:
  - Consolidate duplicated CI logic and remove unused workflow/script surface.
  - Extract shared runtime helpers into `nils-common` only where the logic is domain-neutral.
  - Expand and normalize `nils-test-support` so test cleanup removes local boilerplate instead of recreating it elsewhere.
  - Remove obsolete code, fixtures, reports, plans, and misplaced docs once no live caller/reference remains.
  - Update canonical docs so they describe only the post-refactor architecture and active contributor flows.
- Out of scope:
  - Adding new end-user features or broad behavior redesigns unrelated to duplication/testability.
  - Preserving historical wrappers, aliases, doc redirects, or compatibility shims solely for legacy reasons.
  - Moving crate-local UX text, warning wording, or exit-code policy into shared crates.

## Assumptions

1. `plan-tooling` remains available for validation, batch analysis, and PR-splitting checks.
2. `scripts/dev/workspace-shared-crate-audit.sh` and `scripts/dev/workspace-test-stale-audit.sh` remain the authoritative discovery inputs
   for shared-helper and stale-test cleanup work.
3. No legacy compatibility layer is required unless a currently referenced CI, release, or runtime entrypoint still depends on it.
4. The workspace coverage gate stays at `>= 85.00%` total line coverage throughout the refactor.

## Success criteria

- GitHub Actions and local required checks resolve through canonical entrypoints with redundant workflow/script logic removed.
- Shared runtime logic is centralized where appropriate, with crate-local adapters preserved only for user-visible UX/policy differences.
- Test harness duplication drops materially because reusable helpers live in `nils-test-support`, and stale/orphan helper inventory shrinks
  without reintroducing deprecated-path leftovers.
- Workspace and crate-local docs reflect the refactored boundaries, and obsolete docs/plans/reports are removed instead of being kept for
  legacy compatibility.
- `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh` and the coverage gate both pass at the
  end of the final sprint.

## Sprint 1: CI entrypoint consolidation and dead-path removal

**Goal**: Collapse CI and local verification flows onto canonical entrypoints, remove duplicated workflow/script logic, and fail fast on
cheap audits before expensive builds. **Demo/Validation**:

- Command(s):
  - `plan-tooling validate --file docs/plans/workspace-refactor-dedup-testability-plan.md`
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv`
  - `bash scripts/dev/workspace-test-stale-audit.sh --format tsv`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
- Verify:
  - Each GitHub workflow step points at a canonical script or is explicitly deleted.
  - Audit/verification ordering is fail-fast and deterministic.
  - Unused CI/dev scripts are removed only after inbound-reference proof.
- **PR grouping intent**: per-sprint
- **Execution Profile**: serial
- Sprint scorecard:
  - `Execution Profile`: serial
  - `TotalComplexity`: 15
  - `CriticalPathComplexity`: 15
  - `MaxBatchWidth`: 1
  - `OverlapHotspots`: `.github/workflows/ci.yml`; `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`; `scripts/ci/`

### Task 1.1: Inventory canonical CI entrypoints and removal criteria

- **Location**:
  - .github/workflows/ci.yml
  - .github/workflows/release.yml
  - .github/workflows/publish-crates.yml
  - DEVELOPMENT.md
  - .agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
  - docs/specs/workspace-ci-entrypoint-inventory-v1.md
- **Description**: Map every workflow job and local required-check command to its canonical owner script, identify duplicated shell
  fragments, and define keep/remove criteria for workflow steps, helper scripts, and generated artifacts, recording the result in
  `docs/specs/workspace-ci-entrypoint-inventory-v1.md`.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Every workflow step has a canonical script owner or a deletion decision.
  - The keep/remove criteria explicitly require active callers in workflows, docs, or contributor entrypoints.
  - The inventory and removal criteria are captured in `docs/specs/workspace-ci-entrypoint-inventory-v1.md`.
- **Validation**:
  - `test -f docs/specs/workspace-ci-entrypoint-inventory-v1.md`
  - `rg -n 'scripts/ci/|nils-cli-verify-required-checks' .github/workflows/ci.yml .github/workflows/release.yml .github/workflows/publish-crates.yml DEVELOPMENT.md .agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `rg -n 'canonical|delete|keep|workflow' docs/specs/workspace-ci-entrypoint-inventory-v1.md`

### Task 1.2: Extract cross-platform verification entrypoints

- **Location**:
  - .github/workflows/ci.yml
  - .agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
  - scripts/ci/docs-placement-audit.sh
  - scripts/ci/test-stale-audit.sh
- **Description**: Move duplicated Linux/macOS verification sequences behind shared shell entrypoints, keeping only platform bootstrap
  differences in workflow YAML and making the required-check script the single source of truth for audit order.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Linux and macOS CI jobs share the same verification contract after setup.
  - The required-check script owns audit ordering instead of duplicated workflow YAML fragments.
- **Validation**:
  - `rg -n 'Nils CLI checks|third-party-artifacts-audit|Completion asset audit' .github/workflows/ci.yml`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`

### Task 1.3: Remove obsolete CI/dev helper surface

- **Location**:
  - scripts/ci/wrapper-mode-smoke.sh
  - scripts/ci/agent-docs-snapshots.sh
  - tests/completion/coverage_matrix.sh
  - tests/third-party-artifacts/release-package.test.sh
  - wrappers/plan-tooling
  - wrappers/git-cli
- **Description**: Delete unreferenced CI/dev helpers, stale baselines, and script paths that no longer participate in the canonical
  workflow, while keeping only the commands that still have a live caller in CI, release, docs, or contributor flows.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Every surviving script has an active caller discoverable by repository search.
  - Deleted helper paths have no remaining workflow/doc/runtime references.
- **Validation**:
  - `rg -n 'scripts/ci/|scripts/dev/|wrappers/' .github/workflows README.md DEVELOPMENT.md docs .agents crates tests`
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`

### Task 1.4: Reorder fail-fast CI gates and coverage boundaries

- **Location**:
  - .github/workflows/ci.yml
  - scripts/ci/docs-hygiene-audit.sh
  - scripts/ci/test-stale-audit.sh
  - scripts/ci/coverage-summary.sh
- **Description**: Ensure cheap audits run before full workspace compilation where feasible, tighten coverage artifact lifecycle, and make
  CI failure messages point to the canonical script responsible for the check.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Docs/stale/asset audits execute before expensive build or test phases where platform requirements allow.
  - Coverage setup/cleanup is explicit and limited to the coverage job.
- **Validation**:
  - `rg -n 'docs-hygiene-audit|test-stale-audit|coverage' .github/workflows/ci.yml`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`

## Sprint 2: Shared runtime crate extraction and crate-local simplification

**Goal**: Remove duplicated runtime logic from production crates by extracting domain-neutral helpers into `nils-common`, while keeping
crate-local UX and parity-sensitive behavior explicit. **Demo/Validation**:

- Command(s):
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv`
  - `cargo test --workspace`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- Verify:
  - Shared-helper boundaries are explicit and documented.
  - Duplicated process/env/fs/auth persistence logic is reduced without moving user-facing copy into shared crates.
- **PR grouping intent**: group
- **Execution Profile**: parallel-x2
- Sprint scorecard:
  - `Execution Profile`: parallel-x2
  - `TotalComplexity`: 18
  - `CriticalPathComplexity`: 13
  - `MaxBatchWidth`: 2
  - `OverlapHotspots`: `crates/nils-common/src/process.rs`; `crates/nils-common/src/env.rs`; `crates/nils-common/src/fs.rs`; `crates/codex-cli/src/auth/`; `crates/gemini-cli/src/auth/`

### Task 2.1: Freeze shared-crate boundaries from audit evidence

- **Location**:
  - scripts/dev/workspace-shared-crate-audit.sh
  - README.md
  - crates/nils-common/README.md
  - crates/nils-test-support/README.md
  - docs/specs/workspace-shared-crate-boundary-v1.md
- **Description**: Use the shared-crate audit outputs to freeze what belongs in `nils-common`, what stays in `nils-term`, and what must
  remain crate-local, then bucket current hotspots into process/env/no-color and fs/path persistence work lanes, recording the boundary
  decisions in `docs/specs/workspace-shared-crate-boundary-v1.md`.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 3
- **Acceptance criteria**:
  - Boundary docs explicitly separate shared primitives from crate-local UX/policy code.
  - Shared-crate hotspot lanes are grouped by extraction theme and owning sprint task.
  - `docs/specs/workspace-shared-crate-boundary-v1.md` captures the keep-local vs extract decisions for the audited hotspot families.
- **Validation**:
  - `test -f docs/specs/workspace-shared-crate-boundary-v1.md`
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv`
  - `rg -n 'What belongs|What stays crate-local|Non-goals' README.md crates/nils-common/README.md crates/nils-test-support/README.md`
  - `rg -n 'keep-local|extract|nils-common|nils-term' docs/specs/workspace-shared-crate-boundary-v1.md`

### Task 2.2: Extract process, env, and no-color primitives into `nils-common`

- **Location**:
  - crates/nils-common/src/process.rs
  - crates/nils-common/src/env.rs
  - crates/nils-common/src/git.rs
  - crates/nils-common/src/clipboard.rs
  - crates/nils-common/src/rate_limits_ansi.rs
  - crates/semantic-commit/src/git.rs
  - crates/semantic-commit/src/commit.rs
  - crates/git-cli/src/commit.rs
  - crates/git-cli/src/util.rs
  - crates/fzf-cli/src/git_commit.rs
  - crates/fzf-cli/src/util.rs
  - crates/git-scope/src/tree.rs
  - crates/memo-cli/src/output/text.rs
  - crates/api-testing-core/src/graphql/schema_file.rs
  - crates/api-testing-core/src/grpc/runner.rs
  - crates/api-testing-core/src/suite/resolve.rs
- **Description**: Consolidate manual command probing, git subprocess plumbing, env mutation handling, and `NO_COLOR` logic into
  `nils-common`, while preserving crate-local formatting, warning wording, and exit-code mapping at the call sites.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Audited process/env/no-color hotspots are handled by shared helpers or explicitly justified as crate-local.
  - Characterization tests keep user-visible and JSON-contract behavior unchanged for touched commands.
- **Validation**:
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv`
  - `cargo test -p nils-common -p semantic-commit -p git-cli -p fzf-cli -p git-scope -p memo-cli -p api-testing-core`
  - `cargo test --workspace`
  - `cargo clippy --all-targets --all-features -- -D warnings`

### Task 2.3: Consolidate provider auth persistence and filesystem primitives

- **Location**:
  - crates/nils-common/src/fs.rs
  - crates/nils-common/src/provider_runtime/paths.rs
  - crates/codex-cli/src/auth/current.rs
  - crates/codex-cli/src/auth/save.rs
  - crates/codex-cli/src/auth/refresh.rs
  - crates/codex-cli/src/auth/remove.rs
  - crates/codex-cli/src/auth/sync.rs
  - crates/codex-cli/src/fs.rs
  - crates/codex-cli/src/paths.rs
  - crates/codex-cli/src/provider_profile.rs
  - crates/codex-cli/src/rate_limits/mod.rs
  - crates/gemini-cli/src/auth/current.rs
  - crates/gemini-cli/src/auth/save.rs
  - crates/gemini-cli/src/auth/refresh.rs
  - crates/gemini-cli/src/auth/remove.rs
  - crates/gemini-cli/src/auth/sync.rs
  - crates/gemini-cli/src/fs.rs
  - crates/gemini-cli/src/config.rs
  - crates/gemini-cli/src/provider_profile.rs
  - crates/gemini-cli/src/rate_limits/mod.rs
- **Description**: Pull atomic-write and shared path-resolution primitives into `nils-common` or `provider_runtime` where they are truly
  domain-neutral, characterize secret-dir behavior before extraction, and replace duplicated Codex/Gemini persistence code with thin
  crate-local adapters over shared building blocks.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Codex/Gemini persistence code shares one audited substrate for atomic IO and common path logic.
  - Parity-sensitive secret-dir and provider-specific UX behavior stays adapter-local when full unification would be unsafe.
  - Redundant crate-local filesystem helpers are removed after migration.
- **Validation**:
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv`
  - `cargo test -p nils-common -p codex-cli -p gemini-cli`
  - `cargo test --workspace`
  - `cargo clippy --all-targets --all-features -- -D warnings`

### Task 2.4: Delete redundant crate-local wrappers and re-document ownership

- **Location**:
  - README.md
  - crates/nils-common/README.md
  - crates/nils-test-support/README.md
  - crates/nils-term/docs/README.md
  - crates/codex-cli/src/fs.rs
  - crates/gemini-cli/src/fs.rs
  - crates/git-cli/src/util.rs
- **Description**: Remove wrapper modules and helper functions that became redundant after shared-crate extraction, keep `nils-term`
  focused on terminal UX only, and update crate docs to reflect the simplified ownership boundaries.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Replaced helper paths no longer linger as dead wrappers or duplicate shims.
  - Shared crate docs match the final ownership model and do not advertise removed helpers.
- **Validation**:
  - `bash scripts/dev/workspace-shared-crate-audit.sh --format tsv`
  - `cargo test --workspace`
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`

## Sprint 3: Test cleanup and `nils-test-support` convergence

**Goal**: Reduce test duplication by strengthening `nils-test-support`, removing obsolete helpers and fixtures, and serializing the
highest-overlap crates to avoid cleanup regressions. **Demo/Validation**:

- Command(s):
  - `bash scripts/dev/workspace-test-stale-audit.sh --format tsv`
  - `bash scripts/ci/test-stale-audit.sh --strict`
  - `cargo test --workspace`
- Verify:
  - Reusable test helpers live in `nils-test-support` instead of per-crate harnesses.
  - Orphan helpers, `allow(dead_code)` leftovers, and obsolete fixtures are removed only with replacement coverage evidence.
- **PR grouping intent**: group
- **Execution Profile**: parallel-x2
- Sprint scorecard:
  - `Execution Profile`: parallel-x2
  - `TotalComplexity`: 19
  - `CriticalPathComplexity`: 14
  - `MaxBatchWidth`: 2
  - `OverlapHotspots`: `crates/nils-test-support/src/lib.rs`; `crates/git-cli/tests/`; `crates/agent-docs/tests/`; `crates/macos-agent/tests/`; `crates/fzf-cli/tests/`; `crates/memo-cli/tests/`

### Task 3.1: Freeze stale-test cleanup rules and crate sequencing

- **Location**:
  - scripts/dev/workspace-test-stale-audit.sh
  - scripts/ci/test-stale-audit.sh
  - scripts/ci/test-stale-audit-baseline.tsv
  - docs/runbooks/test-cleanup-governance.md
  - docs/specs/workspace-test-cleanup-lane-matrix-v1.md
- **Description**: Convert current stale-test audit outputs into a deterministic cleanup map, preserve explicit `remove|rewrite|keep|defer`
  decision rules, and formalize the serial-group crate order for the highest-overlap test suites in
  `docs/specs/workspace-test-cleanup-lane-matrix-v1.md`.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 3
- **Acceptance criteria**:
  - Cleanup rules cite current audit artifacts and serial-group order.
  - Baseline update policy is explicit and does not permit silent regression hiding.
  - `docs/specs/workspace-test-cleanup-lane-matrix-v1.md` captures crate tiers, serial groups, and removal/rewrite rules.
- **Validation**:
  - `test -f docs/specs/workspace-test-cleanup-lane-matrix-v1.md`
  - `bash scripts/dev/workspace-test-stale-audit.sh --format tsv`
  - `bash scripts/ci/test-stale-audit.sh --strict`
  - `rg -n 'remove|rewrite|defer|serial' docs/runbooks/test-cleanup-governance.md`
  - `rg -n 'serial|parallel|remove|rewrite|defer' docs/specs/workspace-test-cleanup-lane-matrix-v1.md`

### Task 3.2: Expand `nils-test-support` with missing reusable harness primitives

- **Location**:
  - crates/nils-test-support/src/lib.rs
  - crates/nils-test-support/src/cmd.rs
  - crates/nils-test-support/src/git.rs
  - crates/nils-test-support/src/fs.rs
  - crates/nils-test-support/src/bin.rs
  - crates/nils-test-support/src/http.rs
  - crates/nils-test-support/src/fixtures.rs
  - crates/nils-test-support/tests/guards.rs
  - crates/nils-test-support/tests/bin_cmd.rs
  - crates/nils-test-support/tests/git.rs
  - crates/nils-test-support/tests/fs.rs
  - crates/nils-test-support/tests/http.rs
  - crates/nils-test-support/tests/fixtures.rs
  - crates/nils-test-support/README.md
- **Description**: Add or normalize env/path/bin/git/fs helpers needed by the audit hotspots so consumer crates can delete local harness
  code rather than copying it again, while keeping contract-specific assertions local to each crate.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `nils-test-support` covers the env guard, PATH prepend, executable chmod, binary resolution, and git setup patterns identified by the
    current audit.
  - Tests and README show the canonical helper usage for future migrations.
- **Validation**:
  - `cargo test -p nils-test-support`
  - `bash scripts/dev/workspace-test-stale-audit.sh --format tsv`
  - `bash scripts/ci/test-stale-audit.sh --strict`

### Task 3.3: Migrate parallel-safe crates to shared test helpers

- **Location**:
  - crates/api-gql/tests/history.rs
  - crates/api-grpc/tests/integration.rs
  - crates/api-rest/tests/history.rs
  - crates/api-test/tests/e2e.rs
  - crates/api-testing-core/tests/report_history.rs
  - crates/api-websocket/tests/json_contract.rs
  - crates/git-lock/tests/diff_tag.rs
  - crates/git-scope/tests/rendering.rs
  - crates/git-summary/tests/summary_counts.rs
  - crates/image-processing/tests/core_flows.rs
  - crates/nils-common/tests/provider_runtime_contract.rs
  - crates/nils-term/tests/progress.rs
  - crates/plan-issue-cli/tests/runtime_truth_plan_and_sprint_flow.rs
  - crates/plan-tooling/tests/split_prs.rs
  - crates/screen-record/tests/linux_unit.rs
  - crates/semantic-commit/tests/staged_context.rs
- **Description**: Migrate low/medium-risk test harness code to `nils-test-support`, delete orphan helpers and unnecessary
  `allow(dead_code)` allowances, and keep command-contract assertions local to the owning crates.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Parallel-safe crates stop duplicating env/bin/git harness patterns already covered by `nils-test-support`.
  - Removed helpers are either truly orphaned or replaced by explicit shared-helper usage backed by tests.
- **Validation**:
  - `bash scripts/dev/workspace-test-stale-audit.sh --format tsv`
  - `bash scripts/ci/test-stale-audit.sh --strict`
  - `cargo test --workspace`

### Task 3.4: Serialize high-overlap test crates and remove obsolete fixtures

- **Location**:
  - crates/git-cli/tests/common.rs
  - crates/git-cli/src/util.rs
  - crates/agent-docs/tests/common.rs
  - crates/agent-docs/src/path.rs
  - crates/macos-agent/tests/common.rs
  - crates/macos-agent/src/test_mode.rs
  - crates/fzf-cli/tests/common.rs
  - crates/fzf-cli/src/util.rs
  - crates/memo-cli/tests/json_contract.rs
  - crates/memo-cli/src/output/text.rs
- **Description**: Handle the five serial-group crates one lane at a time, removing obsolete fixtures, unused helpers, and stale
  allowances only after characterization confirms the surviving tests still protect active behavior and machine contracts.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Serial-group crates no longer rely on orphan helpers or dead-code allowances for outdated paths.
  - Replacement coverage is explicit whenever a removed fixture/helper previously guarded user-visible behavior.
- **Validation**:
  - `bash scripts/dev/workspace-test-stale-audit.sh --format tsv`
  - `bash scripts/ci/test-stale-audit.sh --strict`
  - `cargo test --workspace`

## Sprint 4: Documentation consolidation and final dead-surface removal

**Goal**: Make the documentation tree match the simplified codebase, remove obsolete plans/reports/specs, and complete the refactor with
full verification and coverage. **Demo/Validation**:

- Command(s):
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - Root docs contain only living workspace-level material.
  - Crate-local docs live under each owning crate's `docs/` directory.
  - Obsolete docs and dead code paths are removed with no legacy redirects retained unless a live caller still needs them.
- **PR grouping intent**: per-sprint
- **Execution Profile**: serial
- Sprint scorecard:
  - `Execution Profile`: serial
  - `TotalComplexity`: 16
  - `CriticalPathComplexity`: 16
  - `MaxBatchWidth`: 1
  - `OverlapHotspots`: `README.md`; `DEVELOPMENT.md`; `docs/`; `crates/*/docs/`; `scripts/ci/docs-placement-audit.sh`

### Task 4.1: Classify living docs and build the deletion list

- **Location**:
  - README.md
  - DEVELOPMENT.md
  - BINARY_DEPENDENCIES.md
  - docs/plans/third-party-licenses-notices-release-packaging-plan.md
  - docs/reports/completion-coverage-matrix.md
  - docs/runbooks/test-cleanup-governance.md
  - docs/specs/crate-docs-placement-policy.md
  - crates/nils-common/docs/README.md
  - crates/nils-test-support/docs/README.md
  - scripts/ci/docs-placement-audit.sh
  - scripts/ci/docs-hygiene-audit.sh
  - docs/specs/workspace-doc-retention-matrix-v1.md
- **Description**: Classify every surviving document as workspace-level, crate-local, or transient-dev-record, then build a deletion list
  for obsolete docs, reports, and plan artifacts that no longer have a live caller or governance need, recording the result in
  `docs/specs/workspace-doc-retention-matrix-v1.md`.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 3
- **Acceptance criteria**:
  - Every kept doc has an owning scope and a reason to remain.
  - Every doc slated for removal has inbound-reference proof showing no active dependency remains.
  - `docs/specs/workspace-doc-retention-matrix-v1.md` records ownership, lifecycle state, and delete/keep rationale for audited docs.
- **Validation**:
  - `test -f docs/specs/workspace-doc-retention-matrix-v1.md`
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
  - `find docs crates -path '*/docs/*' -type f | sort`
  - `rg -n 'workspace-level|crate-local|transient-dev-record|delete|keep' docs/specs/workspace-doc-retention-matrix-v1.md`

### Task 4.2: Rewrite canonical docs for the simplified architecture

- **Location**:
  - README.md
  - DEVELOPMENT.md
  - BINARY_DEPENDENCIES.md
  - crates/nils-common/README.md
  - crates/nils-test-support/README.md
  - crates/codex-cli/docs/README.md
  - crates/gemini-cli/docs/README.md
  - crates/plan-tooling/docs/README.md
  - crates/plan-issue-cli/docs/README.md
- **Description**: Update the repo narrative so CI entrypoints, shared-helper boundaries, and test-helper usage match the refactored
  implementation, and ensure no removed paths remain documented as active.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace docs point only to canonical entrypoints and living crate paths.
  - Shared-helper docs describe the extracted APIs and their non-goals accurately.
- **Validation**:
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
  - `rg -n 'nils-common|nils-test-support|workspace-shared-crate-audit|workspace-test-stale-audit' README.md DEVELOPMENT.md BINARY_DEPENDENCIES.md crates/nils-common/README.md crates/nils-test-support/README.md`

### Task 4.3: Remove obsolete plans, reports, misplaced docs, and compatibility-only text

- **Location**:
  - docs/plans/markdown-gh-handling-audit-remediation-plan.md
  - docs/plans/third-party-licenses-notices-release-packaging-plan.md
  - docs/reports/completion-coverage-matrix.md
  - docs/runbooks/wrappers-mode-usage.md
  - docs/specs/markdown-github-handling-audit-v1.md
  - crates/plan-tooling/docs/runbooks/split-prs-migration.md
  - crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md
- **Description**: Delete outdated plan files, superseded reports, crate-owned docs misplaced under root `docs/`, and compatibility-only
  wording or redirect material that is no longer required by a live script, workflow, or documented contributor flow.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Dead docs are removed instead of stubbed when no caller remains.
  - Root `docs/` contains only workspace-level material after cleanup.
  - Active docs no longer reference removed compatibility-only paths.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
  - `rg -n --hidden --glob '!.git' '\\blegacy\\b' README.md DEVELOPMENT.md docs crates`

### Task 4.4: Final dead-surface sweep and baseline lock

- **Location**:
  - Cargo.toml
  - crates/nils-common/Cargo.toml
  - crates/nils-test-support/Cargo.toml
  - crates/codex-cli/Cargo.toml
  - crates/gemini-cli/Cargo.toml
  - .github/workflows/ci.yml
  - scripts/ci/test-stale-audit.sh
  - scripts/dev/workspace-shared-crate-audit.sh
  - README.md
  - DEVELOPMENT.md
- **Description**: Remove leftover dependencies, dead modules, scripts, and docs discovered after the doc rewrite, then run the full
  required checks and coverage gate to lock the new simplified baseline.
- **Dependencies**:
  - Task 4.2
  - Task 4.3
- **Complexity**: 4
- **Acceptance criteria**:
  - No replaced flow remains half-removed through dead dependencies or dangling references.
  - Required checks and coverage pass on the simplified workspace.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - `rg -n --hidden --glob '!.git' 'workspace-ci-entrypoint-inventory-v1|workspace-shared-crate-boundary-v1|workspace-test-cleanup-lane-matrix-v1|workspace-doc-retention-matrix-v1' README.md DEVELOPMENT.md docs crates .github scripts`

## Testing Strategy

- Unit:
  - Add or expand deterministic tests around extracted helpers in `nils-common` and `nils-test-support`.
- Integration:
  - Re-run touched crate test suites plus `cargo test --workspace` after each sprint gate.
  - Keep output/JSON contract assertions in the consuming crates when shared logic moves underneath them.
- E2E/manual:
  - Re-run the canonical required-check entrypoint after CI/doc/test boundary changes.
  - Re-run shared-crate and stale-test audits after each cleanup wave to confirm hotspot and orphan reductions.
- Coverage:
  - Use the workspace `cargo llvm-cov nextest` gate after non-doc changes and do not accept refactors that lower total line coverage
    below `85.00%`.

## Risks & gotchas

- `codex-cli` and `gemini-cli` have parity-sensitive secret-dir and auth-persistence behavior; over-eager extraction can create subtle
  regressions if characterization comes after the move.
- The stale-test audit shows high-overlap serial groups (`git-cli`, `agent-docs`, `macos-agent`, `fzf-cli`, `memo-cli`); treating them as
  parallel cleanup work is likely to cause merge friction and accidental coverage loss.
- Doc cleanup can break CI or release flows if a path still has an inbound reference; every removal needs reference proof, not only human
  judgment.
- Linux/macOS workflow differences should stay limited to bootstrap/setup; if platform-specific verification logic leaks back into the YAML,
  CI duplication will regress quickly.

## Rollback plan

- Roll back by sprint boundary, not by mixing partial reversions across the whole repo. Each sprint should land only after its own
  validation gate is green, so reverting the last sprint PR should restore a known-good state.
- If shared-helper extraction causes contract drift, restore the crate-local adapter implementation first, keep the characterization tests,
  and retry the shared boundary with a narrower helper API instead of reintroducing broad duplication.
- If test cleanup removes a still-needed fixture/helper, restore the smallest local helper necessary in the affected crate, re-run the
  stale-test audit, and only retry removal after replacement coverage is explicit.
- If doc cleanup removes a path that a live workflow still references, restore the canonical doc content or update the caller in the same
  rollback PR; do not resurrect whole historical doc trees for compatibility alone.
