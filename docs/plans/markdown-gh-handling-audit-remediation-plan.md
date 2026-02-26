# Plan: Workspace Markdown Handling Audit and gh Write Hardening

## Overview
This plan inventories markdown handling across all existing production Rust source files and remediates every discovered gap in a serial, sprint-gated flow. The repository keeps GitHub operations on the current `gh` CLI path and explicitly does not introduce `crates/nils-common/src/github.rs`. The implementation focuses on shared markdown helper consistency, guarded markdown write paths, and contract-preserving migrations. Each sprint must pass its own validation gate before the next sprint starts.

## Scope
- In scope:
  - Inventory all markdown and GitHub markdown-write touchpoints under `crates/**/src/**/*.rs`.
  - Define a machine-checkable audit status model and close every production row.
  - Harden shared markdown helper behavior and migrate existing callsites to that contract.
  - Keep `gh` adapters crate-local and enforce the boundary with explicit checks.
- Out of scope:
  - Replacing `gh` with direct GitHub API clients.
  - Adding `nils-common::github` or `crates/nils-common/src/github.rs`.
  - Refactoring unrelated output UX or command behavior.

## Assumptions
1. `plan-tooling` remains available for plan parsing/validation/splitting checks.
2. `gh` is available where live GitHub issue/PR operations are executed.
3. Existing output contracts stay stable unless updated in paired specs/tests.
4. Required checks from `DEVELOPMENT.md` remain mandatory pre-delivery gates.

## Success criteria
- Audit file includes all production markdown/GitHub-write touchpoints with `status=open|resolved`.
- Final audit contains zero `status=open` rows.
- Markdown helper behavior is centralized by contract and covered by regression tests.
- GitHub boundary is enforced: crate-local `gh` adapters only, no `nils-common/src/github.rs`.

## Sprint 1: Workspace-wide inventory and boundary contract
**Goal**: Define strict audit boundaries and produce complete, machine-checkable inventory + ownership rules.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/markdown-gh-handling-audit-remediation-plan.md`
  - `rg -n --glob 'crates/**/src/**/*.rs' 'render_.*markdown|markdown::|validate_markdown_payload|canonicalize_table_cell|code_block\(|heading\(|write_temp_markdown|body-file|run_output\("gh"' crates`
  - `test ! -f crates/nils-common/src/github.rs`
- Verify:
  - Audit boundaries and row schema are locked and reproducible.
  - Inventory covers all discovered source-level touchpoints with explicit owners.
  - GitHub boundary decision is documented and guarded.
- **PR grouping intent**: `per-sprint`
- **Execution Profile**: `serial`
- Sprint scorecard: `ExecutionProfile=serial`, `TotalComplexity=14`, `CriticalPathComplexity=14`, `MaxBatchWidth=1`, `OverlapHotspots=docs/specs/markdown-github-handling-audit-v1.md; crates/nils-common/docs/specs/markdown-helpers-contract-v1.md`.

### Task 1.1: Define inventory boundary and audit row schema
- **Location**:
  - docs/specs/markdown-github-handling-audit-v1.md
  - docs/plans/markdown-gh-handling-audit-remediation-plan.md
- **Description**: Define inventory rules (`crates/**/src/**/*.rs` include scope, exclusions, and risk classes) and lock a markdown table schema with required columns including `status=open|resolved` and `test_ref=...`.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Audit spec includes include/exclude rules and risk class definitions.
  - Audit schema requires `crate`, `file`, `function`, `risk_class`, `owner`, `status`, and `test_ref` columns.
- **Validation**:
  - `test -f docs/specs/markdown-github-handling-audit-v1.md`
  - `rg -n 'status=(open|resolved)|test_ref=|risk_class|crates/\*\*/src/\*\*/\*.rs' docs/specs/markdown-github-handling-audit-v1.md`

### Task 1.2: Populate complete source-level markdown/GitHub-write inventory
- **Location**:
  - docs/specs/markdown-github-handling-audit-v1.md
  - crates/nils-common/src/markdown.rs
  - crates/plan-issue-cli/src/execute.rs
  - crates/plan-issue-cli/src/github.rs
  - crates/plan-issue-cli/src/issue_body.rs
  - crates/plan-issue-cli/src/task_spec.rs
  - crates/api-testing-core/src/markdown.rs
  - crates/api-testing-core/src/report.rs
  - crates/api-testing-core/src/suite/summary.rs
  - crates/api-rest/src/commands/report.rs
  - crates/api-gql/src/commands/report.rs
  - crates/api-grpc/src/commands/report.rs
  - crates/api-websocket/src/commands/report.rs
  - crates/memo-cli/src/preprocess/detect.rs
  - crates/memo-cli/src/preprocess/validate.rs
  - crates/plan-tooling/src/parse.rs
  - crates/plan-tooling/src/validate.rs
  - crates/plan-tooling/src/split_prs.rs
- **Description**: Run source-only discovery and capture every production touchpoint row in the audit table with owner and initial `status=open` or `status=resolved`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Inventory includes all matches from defined source-discovery commands.
  - Every row has a non-empty owner and valid status value.
- **Validation**:
  - `rg -n --glob 'crates/**/src/**/*.rs' 'render_.*markdown|validate_markdown_payload|canonicalize_table_cell|write_temp_markdown|run_output\("gh"|parse_markdown_row|parse_sprint_heading|render_summary_markdown' crates`
  - `set -euo pipefail; for f in crates/nils-common/src/markdown.rs crates/plan-issue-cli/src/execute.rs crates/plan-issue-cli/src/github.rs crates/plan-issue-cli/src/issue_body.rs crates/plan-issue-cli/src/task_spec.rs crates/api-testing-core/src/markdown.rs crates/api-testing-core/src/report.rs crates/api-testing-core/src/suite/summary.rs crates/api-rest/src/commands/report.rs crates/api-gql/src/commands/report.rs crates/api-grpc/src/commands/report.rs crates/api-websocket/src/commands/report.rs crates/memo-cli/src/preprocess/detect.rs crates/memo-cli/src/preprocess/validate.rs crates/plan-tooling/src/parse.rs crates/plan-tooling/src/validate.rs crates/plan-tooling/src/split_prs.rs; do rg -n --fixed-strings \"$f\" docs/specs/markdown-github-handling-audit-v1.md >/dev/null; done`
  - `rg -n 'status=(open|resolved)' docs/specs/markdown-github-handling-audit-v1.md`

### Task 1.3: Map inventory rows to concrete regression coverage
- **Location**:
  - docs/specs/markdown-github-handling-audit-v1.md
  - crates/nils-common/tests/markdown_table_canonicalization.rs
  - crates/plan-issue-cli/tests/live_issue_ops.rs
  - crates/plan-issue-cli/tests/parity_guardrails.rs
  - crates/api-testing-core/tests/report_history.rs
- **Description**: For each inventory row, map at least one existing or planned regression test and mark rows requiring new coverage.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Each audit row references at least one test target.
  - Rows lacking coverage are explicitly marked with planned test file and command.
- **Validation**:
  - `cargo test -p nils-common --test markdown_table_canonicalization`
  - `cargo test -p nils-plan-issue-cli --test live_issue_ops`
  - `cargo test -p nils-api-testing-core --test report_history`
  - `rg -n 'test_ref=' docs/specs/markdown-github-handling-audit-v1.md`
  - `test -f docs/specs/markdown-github-handling-audit-v1.md && ! rg -n 'test_ref=missing' docs/specs/markdown-github-handling-audit-v1.md`

### Task 1.4: Lock GitHub boundary and enforce crate ownership rules
- **Location**:
  - crates/nils-common/README.md
  - crates/nils-common/docs/specs/markdown-helpers-contract-v1.md
  - crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v2.md
  - docs/specs/markdown-github-handling-audit-v1.md
- **Description**: Document and enforce that GitHub writes stay on crate-local `gh` adapters and shared markdown stays in `nils-common` without adding a shared GitHub module.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Docs explicitly state “keep `gh`, no `nils-common/src/github.rs`”.
  - Allowed `gh` owners in production source are explicitly listed.
- **Validation**:
  - `test ! -f crates/nils-common/src/github.rs`
  - `rg -n --glob 'crates/**/src/**/*.rs' 'run_output\("gh"' crates`
  - `rg -n 'no `nils-common/src/github.rs`|crate-local `gh`' crates/nils-common/README.md crates/nils-common/docs/specs/markdown-helpers-contract-v1.md crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v2.md docs/specs/markdown-github-handling-audit-v1.md`

## Sprint 2: Shared helper hardening and serial callsite migration
**Goal**: Implement shared markdown helper improvements and migrate major callsites in controlled serial steps.
**Demo/Validation**:
- Command(s):
  - `plan-tooling to-json --file docs/plans/markdown-gh-handling-audit-remediation-plan.md --sprint 2 --pretty`
  - `cargo test -p nils-common`
  - `cargo test -p nils-api-testing-core --test report_history`
  - `cargo test -p nils-plan-issue-cli --test live_issue_ops`
- Verify:
  - Shared helper contract is implemented and consumed by core callsites.
  - Migration preserves output/guard behavior under existing tests.
- **PR grouping intent**: `per-sprint`
- **Execution Profile**: `serial`
- Sprint scorecard: `ExecutionProfile=serial`, `TotalComplexity=16`, `CriticalPathComplexity=16`, `MaxBatchWidth=1`, `OverlapHotspots=crates/nils-common/src/markdown.rs; crates/api-testing-core/src/markdown.rs; crates/plan-issue-cli/src/github.rs`.

### Task 2.1: Extend `nils-common::markdown` contract and tests
- **Location**:
  - crates/nils-common/src/markdown.rs
  - crates/nils-common/src/lib.rs
  - crates/nils-common/tests/markdown_table_canonicalization.rs
  - crates/nils-common/docs/specs/markdown-helpers-contract-v1.md
- **Description**: Extend shared markdown primitives needed by audited callsites while preserving current payload and canonicalization semantics.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 5
- **Acceptance criteria**:
  - New helper APIs are documented and covered by unit/integration tests.
  - Existing behavior for escaped-control guard and table canonicalization is unchanged.
- **Validation**:
  - `cargo test -p nils-common`

### Task 2.2: Migrate `api-testing-core` report builders to shared helper usage
- **Location**:
  - crates/api-testing-core/src/markdown.rs
  - crates/api-testing-core/src/report.rs
  - crates/api-testing-core/src/rest/report.rs
  - crates/api-testing-core/src/graphql/report.rs
  - crates/api-testing-core/src/grpc/report.rs
  - crates/api-testing-core/src/websocket/report.rs
  - crates/api-testing-core/tests/report_history.rs
- **Description**: Align report markdown rendering with shared helper contract and remove drift-prone duplicate behavior.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `api-testing-core` report markdown output remains contract-stable.
  - Duplicate markdown rendering drift points are removed or wrapped by a thin stable facade.
- **Validation**:
  - `cargo test -p nils-api-testing-core --test report_history`
  - `cargo test -p nils-api-testing-core --test cli_report`

### Task 2.3: Migrate API command report entrypoints to updated markdown flow
- **Location**:
  - crates/api-rest/src/commands/report.rs
  - crates/api-gql/src/commands/report.rs
  - crates/api-grpc/src/commands/report.rs
  - crates/api-websocket/src/commands/report.rs
- **Description**: Apply shared markdown flow consistently in API command crates and keep command-level output contracts unchanged.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - All API command report entrypoints use the updated shared markdown path.
  - Existing command report tests remain green.
- **Validation**:
  - `cargo test -p nils-api-rest --tests`
  - `cargo test -p nils-api-gql --tests`
  - `cargo test -p nils-api-grpc --tests`
  - `cargo test -p nils-api-websocket --tests`

### Task 2.4: Harden `plan-issue-cli` markdown guard paths for body/comment writes
- **Location**:
  - crates/plan-issue-cli/src/github.rs
  - crates/plan-issue-cli/src/execute.rs
  - crates/plan-issue-cli/src/render.rs
  - crates/plan-issue-cli/src/issue_body.rs
  - crates/plan-issue-cli/src/task_spec.rs
- **Description**: Ensure all `plan-issue-cli` markdown body/comment generation and write paths consistently apply shared guard semantics, including strict vs force-mode behavior.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Strict mode rejects escaped-control artifacts on all write paths.
  - Force mode continues to bypass only contract-allowed paths.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test live_issue_ops`
  - `cargo test -p nils-plan-issue-cli --test parity_guardrails`
  - `cargo test -p nils-plan-issue-cli --test task_spec_flow`

## Sprint 3: Closure, split executability checks, and delivery gates
**Goal**: Close audit rows, prove plan executability deterministically, and pass repo-level required checks.
**Demo/Validation**:
- Command(s):
  - `plan-tooling batches --file docs/plans/markdown-gh-handling-audit-remediation-plan.md --sprint 3 --format text`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - Inventory is fully resolved (`status=open` count is zero).
  - Executability checks pass for per-sprint deterministic split mapping.
  - Repository required checks and coverage gate pass.
- **PR grouping intent**: `per-sprint`
- **Execution Profile**: `serial`
- Sprint scorecard: `ExecutionProfile=serial`, `TotalComplexity=15`, `CriticalPathComplexity=15`, `MaxBatchWidth=1`, `OverlapHotspots=docs/specs/markdown-github-handling-audit-v1.md; crates/plan-issue-cli/tests/live_issue_ops.rs; crates/api-testing-core/tests/report_history.rs`.

### Task 3.1: Close remaining markdown touchpoints in secondary crates
- **Location**:
  - crates/memo-cli/src/preprocess/detect.rs
  - crates/memo-cli/src/preprocess/validate.rs
  - crates/plan-tooling/src/parse.rs
  - crates/plan-tooling/src/validate.rs
  - crates/plan-tooling/src/split_prs.rs
  - docs/specs/markdown-github-handling-audit-v1.md
- **Description**: Resolve or explicitly classify remaining secondary markdown touchpoints found during the full-source audit and update their row status.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Secondary-crate rows are marked `status=resolved` or documented as non-production exclusions per schema rules.
  - No unresolved production row remains in the audit table.
- **Validation**:
  - `test -f docs/specs/markdown-github-handling-audit-v1.md && ! rg -n 'status=open' docs/specs/markdown-github-handling-audit-v1.md`

### Task 3.2: Add explicit regression suites for markdown-safe GitHub lifecycle
- **Location**:
  - crates/plan-issue-cli/tests/live_issue_ops.rs
  - crates/plan-issue-cli/tests/live_start_sprint_runtime_truth.rs
  - crates/plan-issue-cli/tests/runtime_truth_plan_and_sprint_flow.rs
  - crates/plan-issue-cli/tests/auto_single_lane_runtime_truth.rs
  - crates/api-testing-core/tests/report_history.rs
- **Description**: Add/upgrade explicit integration regressions to verify strict rejection, force bypass, and markdown output stability across lifecycle flows.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Listed integration suites validate markdown lifecycle behaviors with deterministic assertions.
  - Coverage matrix rows tied to these suites are marked resolved.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test live_issue_ops`
  - `cargo test -p nils-plan-issue-cli --test live_start_sprint_runtime_truth`
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`
  - `cargo test -p nils-api-testing-core --test report_history`

### Task 3.3: Prove split executability with deterministic per-sprint checks
- **Location**:
  - docs/plans/markdown-gh-handling-audit-remediation-plan.md
- **Description**: Execute deterministic split checks for every sprint and assert split record counts match task counts for this plan.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 2
- **Acceptance criteria**:
  - `split-prs` deterministic checks pass for sprints 1-3 with per-sprint grouping.
  - Record counts match expected task counts for each sprint.
- **Validation**:
  - `test "$(plan-tooling split-prs --file docs/plans/markdown-gh-handling-audit-remediation-plan.md --scope sprint --sprint 1 --pr-grouping per-sprint --strategy deterministic --format json | rg -o '"task_id"' | wc -l | tr -d ' ')" -eq 4`
  - `test "$(plan-tooling split-prs --file docs/plans/markdown-gh-handling-audit-remediation-plan.md --scope sprint --sprint 2 --pr-grouping per-sprint --strategy deterministic --format json | rg -o '"task_id"' | wc -l | tr -d ' ')" -eq 4`
  - `test "$(plan-tooling split-prs --file docs/plans/markdown-gh-handling-audit-remediation-plan.md --scope sprint --sprint 3 --pr-grouping per-sprint --strategy deterministic --format json | rg -o '"task_id"' | wc -l | tr -d ' ')" -eq 4`

### Task 3.4: Run required gates and publish final closure summary
- **Location**:
  - docs/specs/markdown-github-handling-audit-v1.md
  - docs/plans/markdown-gh-handling-audit-remediation-plan.md
  - DEVELOPMENT.md
- **Description**: Execute full required checks and coverage, then publish closure summary confirming zero open rows and boundary compliance (`gh` retained, no shared GitHub module).
- **Dependencies**:
  - Task 3.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Required checks pass and coverage gate is >= 85%.
  - Closure summary explicitly confirms `test ! -f crates/nils-common/src/github.rs` and zero open rows.
- **Validation**:
  - `test ! -f crates/nils-common/src/github.rs`
  - `test -f docs/specs/markdown-github-handling-audit-v1.md && ! rg -n 'status=open' docs/specs/markdown-github-handling-audit-v1.md`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Testing Strategy
- Unit:
  - `nils-common` markdown helper unit/integration tests.
  - `api-testing-core` markdown report rendering tests.
- Integration:
  - `plan-issue-cli` integration suites for markdown body/comment lifecycle paths.
  - API command crate integration tests for markdown report outputs.
- Executability and gates:
  - `plan-tooling validate`, `to-json`, `batches`, and per-sprint `split-prs` deterministic checks.
  - Required repository checks and coverage gate from `DEVELOPMENT.md`.

## Risks & gotchas
- Shared-helper adoption can alter whitespace/newline behavior if not guarded by output tests.
- Guard strictness changes can break force-mode expectations if bypass boundaries drift.
- Full-source inventory can surface high-volume low-risk rows; exclusion criteria must stay explicit and deterministic.

## Rollback plan
1. Revert Sprint 2 migration commits first (shared-helper adoption and guard rewires), keep Sprint 1 inventory/spec artifacts.
2. Re-run targeted suite commands for `nils-common`, `nils-api-testing-core`, and `nils-plan-issue-cli` to confirm baseline behavior recovery.
3. If needed, restore prior crate-local markdown rendering paths while preserving `gh` adapter ownership rules.
4. Keep audit table and mark rolled-back rows back to `status=open`; restart remediation from last green checkpoint.
