# Plan: split-prs auto sparse scaffold

## Overview
Sparse metadata scaffold for future auto strategy regression tests.

## Scope
- In scope: provide low-density task metadata for auto grouping fallback checks.
- Out of scope: deterministic fixture parity.

## Sprint 1: Sparse metadata

### Task 1.1: Sparse metadata task one
- **Location**:
  - crates/plan-tooling/src/split_prs.rs
- **Description**: Keep sparse metadata path for future auto fallback tests.
- **Dependencies**:
  - none
- **Acceptance criteria**:
  - Sparse fixture remains parseable.
- **Validation**:
  - cargo test -p nils-plan-tooling split_prs_non_regression_auto_sparse_plan_scaffold

### Task 1.2: Sparse metadata task two
- **Location**:
  - crates/plan-tooling/tests/split_prs.rs
- **Description**: Keep sparse metadata path for future tie-break checks.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - Sparse fixture remains stable.
- **Validation**:
  - cargo test -p nils-plan-tooling split_prs_non_regression_auto_sparse_plan_scaffold
