# Plan: plan-tooling split-prs auto strategy

## Overview
This plan adds the `split-prs --strategy auto` path while preserving deterministic output
contracts used by issue orchestration. Sprint 1 freezes contract text, scoring rubric definitions,
and manual-regression workflows before runtime changes land. Sprint 2 implements auto grouping and
strategy-specific validation semantics. Sprint 3 finalizes regression coverage, docs, and release
gates.

## Scope
- In scope: `plan-tooling split-prs` auto strategy semantics, deterministic grouping behavior,
  regression tests, and migration/runbook updates.
- Out of scope: changing issue lifecycle policy semantics outside split grouping behavior.

## Assumptions (if any)
1. Plan Format v1 tasks continue to provide `Location`, `Dependencies`, and optional `Complexity`.
2. Downstream issue automation depends on stable `split-prs` TSV/JSON field names and notes keys.
3. Auto grouping must remain deterministic across repeated runs on the same plan file.

## Sprint 1: Auto strategy contract and heuristic design
**Goal**: Freeze CLI semantics and deterministic heuristic rules before changing runtime behavior.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/plan-tooling-split-prs-auto-strategy-plan.md`
  - `rg -n 'strategy auto|Complexity|Location|Dependencies|group' crates/plan-tooling/README.md crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
  - `cargo test -p nils-plan-tooling split_prs_auto_not_implemented`
- Verify:
  - Contract text explicitly describes deterministic tie-break rules and mode-specific behavior.
  - Existing not-implemented test is preserved as baseline until Sprint 2 lands.

### Task 1.1: Define auto strategy CLI contract and constraints
- **Location**:
  - crates/plan-tooling/README.md
  - crates/plan-tooling/docs/specs/split-prs-contract-v1.md
  - crates/plan-tooling/docs/runbooks/split-prs-migration.md
- **Description**: Document exact auto semantics for `per-sprint` and `group`, including deterministic ordering, expected error surface, and compatibility guarantees for existing schema.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Docs define when `--pr-group` mappings are required and when auto mappings are generated.
  - Contract states deterministic ordering rules for generated groups and output records.
  - Migration notes include deterministic rollback command path.
- **Validation**:
  - test -f crates/plan-tooling/docs/specs/split-prs-contract-v1.md
  - rg -n 'strategy=auto|deterministic ordering|pr-grouping=group|rollback' crates/plan-tooling/docs/specs/split-prs-contract-v1.md crates/plan-tooling/docs/runbooks/split-prs-migration.md
  - rg -n 'split-prs|strategy auto' crates/plan-tooling/README.md

### Task 1.2: Define scoring rubric and deterministic tie-break algorithm
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
  - crates/plan-tooling/tests/split_prs.rs
- **Description**: Define scoring and bucketing rules combining `Complexity`, dependency layers, and `Location` overlap into deterministic auto grouping with stable sparse-field fallbacks.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Auto heuristic contract is documented inline near assignment logic.
  - Tie-break order is deterministic and based on stable keys.
  - Test scaffolding includes sparse-field and overlap-heavy plan shapes.
- **Validation**:
  - cargo test -p nils-plan-tooling split_prs_non_regression
  - cargo test -p nils-plan-tooling split_prs_error_matrix_doc_mentions_core_cases
  - rg -n 'strategy == "auto"|shared-pr-anchor|deterministic' crates/plan-tooling/src/split_prs.rs

### Task 1.3: Define external corpus evaluation harness for manual regression
- **Location**:
  - crates/plan-tooling/tests/split_prs.rs
  - crates/plan-tooling/docs/runbooks/split-prs-build-task-spec-cutover.md
- **Description**: Add a documented manual regression loop for local external plans so heuristic changes can be evaluated without making CI depend on machine-local paths.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Runbook includes no-op-safe conditional local corpus commands.
  - Regression loop captures deterministic output checks and conflict-risk visibility.
  - CI-required checks and local exploratory checks are clearly separated.
- **Validation**:
  - if [ -d /Users/terry/Project/graysurf/nils-cli/docs/plans ]; then find /Users/terry/Project/graysurf/nils-cli/docs/plans -name '*-plan.md' -exec plan-tooling validate --file '{}' ';'; fi
  - if [ -d /Users/terry/Project/graysurf/nils-cli/docs/plans ]; then plan-tooling split-prs --file /Users/terry/Project/graysurf/nils-cli/docs/plans/plan-tooling-split-prs-cutover-plan.md --scope sprint --sprint 2 --pr-grouping per-sprint --strategy deterministic --format json | jq -S . >/dev/null; fi
  - rg -n 'graysurf|local corpus|manual regression' crates/plan-tooling/docs/runbooks/split-prs-build-task-spec-cutover.md

## Sprint 2: Auto runtime implementation
**Goal**: Implement deterministic auto grouping and strategy-aware validation behavior.
**Demo/Validation**:
- Command(s):
  - cargo test -p nils-plan-tooling --test split_prs split_prs_auto_not_implemented -- --exact
  - cargo test -p nils-plan-tooling --test split_prs split_prs_error_group_requires_mapping -- --exact
  - cargo test -p nils-plan-tooling --test split_prs split_prs_non_regression_required_notes_keys -- --exact
- Verify:
  - Auto strategy can assign groups without full manual mapping.
  - Strategy-specific validation behavior remains deterministic.

### Task 2.1: Implement auto assignment engine using task topology and conflict signals
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
  - crates/plan-tooling/tests/split_prs.rs
- **Description**: Implement auto grouping from deterministic scoring and conflict signals derived from plan topology and location overlap.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Auto mode assigns groups without requiring full explicit mappings.
  - Group output is deterministic across repeated runs.
  - Shared-anchor semantics stay stable.
- **Validation**:
  - rg -n '^fn split_prs_auto_group_without_mapping_succeeds\(' crates/plan-tooling/tests/split_prs.rs

### Task 2.2: Wire CLI validation rules for strategy-specific grouping behavior
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
  - crates/plan-tooling/tests/split_prs.rs
- **Description**: Implement strategy-specific validation so deterministic mode keeps strict mapping requirements while auto mode accepts generated mappings.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Deterministic group mode still requires complete `--pr-group` mapping.
  - Auto group mode reports only invalid mapping keys and malformed pins.
  - Usage and runtime errors remain stable.
- **Validation**:
  - cargo test -p nils-plan-tooling --test split_prs split_prs_error_group_requires_mapping -- --exact

### Task 2.3: Emit stable auto notes metadata and shared anchor semantics
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
  - crates/plan-tooling/tests/split_prs.rs
- **Description**: Ensure auto mode preserves stable notes keys and deterministic shared-anchor selection.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Notes output remains backward compatible.
  - Anchor selection for shared groups remains deterministic.
  - Deterministic and auto metadata are diff-friendly.
- **Validation**:
  - cargo test -p nils-plan-tooling --test split_prs split_prs_non_regression_required_notes_keys -- --exact

## Sprint 3: Auto readiness and rollout gates
**Goal**: Lock regression coverage and rollout procedures for release readiness.
**Demo/Validation**:
- Command(s):
  - cargo test -p nils-plan-tooling split_prs
  - cargo test -p nils-plan-tooling completion_outside_repo
  - ./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
- Verify:
  - Regression matrix covers deterministic and auto paths.
  - Docs and completion outputs align with final CLI behavior.
  - Required release checks pass.

### Task 3.1: Add comprehensive auto regression matrix tests
- **Location**:
  - crates/plan-tooling/tests/split_prs.rs
  - crates/plan-tooling/tests/fixtures/split_prs
- **Description**: Add matrix tests for strategy/mode combinations, sparse plans, and overlap-heavy plans with deterministic output assertions.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Test matrix covers happy-path and error-path auto behaviors.
  - Snapshot outputs remain deterministic.
  - Failure messages isolate fixture and strategy quickly.
- **Validation**:
  - cargo test -p nils-plan-tooling split_prs

### Task 3.2: Update completion/help and migration documentation
- **Location**:
  - crates/plan-tooling/src/completion.rs
  - crates/plan-tooling/src/usage.rs
  - crates/plan-tooling/README.md
  - crates/plan-tooling/docs/runbooks/split-prs-migration.md
- **Description**: Align completion/help surfaces and migration guidance with released auto behavior.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Help and completion reflect strategy-specific grouping semantics.
  - Migration runbook documents final rollout and rollback behavior.
  - Docs remain consistent with command output.
- **Validation**:
  - cargo test -p nils-plan-tooling completion_outside_repo

### Task 3.3: Execute release gates and rehearse rollback switchback
- **Location**:
  - DEVELOPMENT.md
  - crates/plan-tooling/docs/runbooks/split-prs-migration.md
  - .agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
- **Description**: Run required checks and exercise rollback instructions before release cutover.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Required checks pass or failures are documented with remediation.
  - Rollback commands are rehearsed with deterministic mode fallback.
  - Release notes capture auto strategy guardrails.
- **Validation**:
  - ./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh

## Testing Strategy
- Unit: deterministic normalization and strategy validation checks in `split_prs.rs`.
- Integration: CLI tests in `crates/plan-tooling/tests/split_prs.rs` with fixture comparisons.
- Manual: optional local corpus regression loop documented in cutover runbook.

## Risks & gotchas
- Auto grouping can increase merge-conflict risk if location overlap weights are poorly tuned.
- Sparse plan metadata may reduce grouping precision; fallbacks must remain deterministic.
- Downstream tooling assumes stable notes keys and header columns.

## Rollback plan
1. Keep deterministic strategy path available at all times.
2. If auto path regresses, switch orchestration back to deterministic command invocation.
3. Re-run fixture parity and local corpus manual regression before re-enabling auto.
