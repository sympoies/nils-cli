# Plan: split-prs auto overlap scaffold

## Overview
Overlap-heavy scaffold for future auto location-aware grouping tests.

## Scope
- In scope: preserve repeatable high-overlap task shapes.
- Out of scope: runtime auto grouping assertions before implementation.

## Sprint 1: Overlap-heavy metadata

### Task 1.1: Backend overlap anchor
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
  - crates/plan-tooling/src/lib.rs
- **Description**: Anchor task with shared backend location paths.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - Overlap fixture remains parseable.
- **Validation**:
  - cargo test -p nils-plan-tooling split_prs_non_regression_auto_overlap_heavy_plan_scaffold

### Task 1.2: Backend overlap pair
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
  - crates/plan-tooling/src/usage.rs
- **Description**: Shared location pair for deterministic tie-break rehearsal.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Overlap fixture ordering remains stable.
- **Validation**:
  - cargo test -p nils-plan-tooling split_prs_non_regression_auto_overlap_heavy_plan_scaffold

### Task 1.3: Docs overlap observer
- **Location**:
  - crates/plan-tooling/docs/specs/split-prs-contract-v1.md
  - crates/plan-tooling/src/split_prs.rs
- **Description**: Blend docs and code paths to simulate overlap conflict risk.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Overlap fixture remains available for future auto evaluation.
- **Validation**:
  - cargo test -p nils-plan-tooling split_prs_non_regression_auto_overlap_heavy_plan_scaffold
