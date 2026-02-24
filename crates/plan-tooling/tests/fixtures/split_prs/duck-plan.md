# Plan: split-prs fixture plan

## Sprint 1: Contract fixtures

### Task 1.1: Define split-prs CLI contract and output schema
- **Location**:
  - crates/plan-tooling/README.md
  - crates/plan-tooling/docs/specs/split-prs-contract-v1.md
- **Description**: Add split-prs deterministic contract details.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Contract file exists.
- **Validation**:
  - test -f crates/plan-tooling/docs/specs/split-prs-contract-v1.md

### Task 1.2: Capture deterministic parity fixtures from current task-spec behavior
- **Location**:
  - crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.tsv
- **Description**: Capture per-sprint expected outputs.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - Per-sprint fixture exists.
- **Validation**:
  - test -f crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.tsv

### Task 1.3: Define deterministic normalization and error matrix
- **Location**:
  - crates/plan-tooling/docs/specs/split-prs-contract-v1.md
- **Description**: Add normalization and error matrix section.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Error matrix includes missing mapping case.
- **Validation**:
  - test -f crates/plan-tooling/docs/specs/split-prs-contract-v1.md

## Sprint 2: Group fixtures

### Task 2.1: Rewire build-task-spec to call plan-tooling split-prs
- **Location**:
  - skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh
- **Description**: Placeholder fixture task for isolated group.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Group fixture task exists.
- **Validation**:
  - test -f crates/plan-tooling/tests/fixtures/split_prs/group_expected.tsv

### Task 2.2: Remove duplicate split logic and keep only orchestration-specific transforms
- **Location**:
  - skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh
- **Description**: Placeholder fixture task for shared group part A.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Shared group task A exists.
- **Validation**:
  - test -f crates/plan-tooling/tests/fixtures/split_prs/group_expected.tsv

### Task 2.3: Prove duck-plan parity for deterministic grouping outputs
- **Location**:
  - skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh
- **Description**: Placeholder fixture task for shared group part B.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Shared group task B exists.
- **Validation**:
  - test -f crates/plan-tooling/tests/fixtures/split_prs/group_expected.tsv
