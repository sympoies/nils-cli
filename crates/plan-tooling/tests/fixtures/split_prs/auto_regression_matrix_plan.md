# Plan: split-prs auto regression matrix fixture

## Overview
Fixture for auto strategy matrix coverage: complexity pressure, external blockers, and location overlap.

## Sprint 1: Auto matrix

### Task 1.1: Build scheduler conflict model
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
  - crates/plan-tooling/src/batches.rs
- **Description**: High-complexity base task that anchors shared overlap heuristics.
- **Dependencies**:
  - none
- **Complexity**: 9
- **Validation**:
  - cargo test -p nils-plan-tooling split_prs

### Task 1.2: Pair scheduler conflict signals with dependency scoring
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
  - crates/plan-tooling/src/batches.rs
- **Description**: Shares locations with Task 1.1 and depends on it.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Validation**:
  - cargo test -p nils-plan-tooling batches

### Task 1.3: Integrate external blocker reconciliation
- **Location**:
  - scripts/ci/docs-hygiene-audit.sh
- **Description**: Includes an unresolved external blocker dependency while still depending on Task 1.1.
- **Dependencies**:
  - EXT-API-BLOCKER
  - Task 1.1
- **Complexity**: 6
- **Validation**:
  - bash scripts/ci/docs-hygiene-audit.sh --strict

### Task 1.4: Publish rollback and migration notes
- **Location**:
  - crates/plan-tooling/docs/runbooks/split-prs-migration.md
- **Description**: Lightweight docs task depending on external blocker integration.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 2
- **Validation**:
  - rg -n 'rollback' crates/plan-tooling/docs/runbooks/split-prs-migration.md
