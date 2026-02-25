# Plan: plan-issue-cli Task Decomposition runtime-truth refactor

## Overview
This plan refactors `crates/plan-issue-cli` so `Task Decomposition` becomes the single runtime-truth execution table for plan/sprint orchestration. It removes the current split between displayed issue rows and runtime task-spec/subagent prompt behavior by making issue creation precompute executable lane metadata and making sprint execution consume the issue table as the source of truth. It also fixes the `--pr-grouping group --strategy auto` single-lane sprint case so `Execution Mode=per-sprint` is paired with normalized lane metadata (`Owner`/`Branch`/`Worktree`/notes) and per-sprint dispatch behavior rather than per-task pseudo-lanes.

## Scope
- In scope:
  - `crates/plan-issue-cli` runtime-truth semantics for `Task Decomposition` (`Owner`, `Branch`, `Worktree`, `Execution Mode`, `Notes`).
  - Auto/group single-lane normalization (`Execution Mode=per-sprint` plus canonical lane metadata and per-sprint execution behavior).
  - `start-plan` issue body generation to precompute and write executable dispatch metadata for the full plan.
  - `start-sprint` execution flow refactor to consume issue rows (not freshly computed task-spec rows) for task-spec artifacts and subagent prompts.
  - Lane-aware prompt/task-spec generation so artifacts match actual execution lanes.
  - Validation/gate updates and regression tests to prevent table/runtime drift.
  - Contract/spec/runbook updates documenting `Task Decomposition` as runtime truth.
- Out of scope:
  - Adding a second issue-body table (for example `Execution Dispatch` / `Dispatch Lanes`) as an additional source of truth.
  - Redesigning the plan markdown parser or `plan-tooling` plan format.
  - Replacing the current task-level status model in `Task Decomposition` (rows remain per-task for status/PR tracking).
  - Changing unrelated GitHub workflow policy semantics outside plan/sprint orchestration.

## Assumptions
1. `Task Decomposition` remains a per-task table; runtime-truth semantics are achieved by canonicalizing lane metadata across rows that share one execution lane.
2. `Owner` in `Task Decomposition` is treated as a stable dispatch owner alias (for example `subagent-s3-t1`), not a platform-internal ephemeral spawned-agent identifier.
3. `plan_tooling::split_prs::build_split_plan_records` remains deterministic and continues emitting `pr_group` plus `shared-pr-anchor=<task_id>` notes for multi-task shared groups, and this dependency is locked by regression coverage in Sprint 2.
4. When `group + auto` converges to one lane in a sprint, execution semantics should match explicit `per-sprint` mode (single runtime lane for the sprint) while task rows remain distinct for status and PR tracking.
5. If the plan file changes after issue creation, the issue table should not be silently overwritten during `start-sprint`; drift must be detected and surfaced explicitly.

## Success criteria
- `start-plan` writes `Task Decomposition` rows with concrete runtime-planned `Owner`/`Branch`/`Worktree`/`Execution Mode`/`Notes` (only `PR` and runtime status fields may remain unresolved).
- `start-sprint` no longer uses freshly computed split rows as the authoritative runtime source for dispatch artifacts; it consumes `Task Decomposition` rows and fails fast on invalid/drifted metadata.
- `task-spec` and subagent prompt artifacts are derived from the same runtime-truth rows and do not contradict the issue table.
- `group + auto` single-lane sprints normalize to `per-sprint` with canonical lane metadata and per-sprint lane dispatch behavior.
- Cleanup and gating logic remain correct when rows share a lane (including branch/worktree cleanup targeting and sprint row selection).

## Sprint 1: Freeze runtime-truth contract and reproduce current drift gaps
**Goal**: Document the runtime-truth semantics and lock failing/characterization coverage for the current display-vs-runtime mismatches before behavior changes.
**PR grouping intent**: `group` (contract/docs lane + regression-tests lane).
**PR group mapping (deterministic validation)**:
- `S1T1=contract`
- `S1T2=tests`
- `S1T3=tests`
**Execution Profile**: `parallel-x2` (parallel width 2).
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/plan-issue-cli-task-decomposition-runtime-truth-plan.md`
  - `cargo test -p nils-plan-issue-cli --test task_spec_flow`
  - `cargo test -p nils-plan-issue-cli sync_issue_rows_from_task_spec_auto_single_group_uses_per_sprint_mode -- --exact`
- Verify:
  - Specs/docs state that `Task Decomposition` is the single runtime-truth table and no second dispatch table is introduced.
  - Characterization tests prove current auto single-lane behavior only normalizes `Execution Mode` and leaves lane metadata inconsistent.
  - Characterization tests capture current `start-plan`/`start-sprint` task-spec-as-source overwrite behavior for later refactor.
**Sprint scorecard**:
- `TotalComplexity`: 15
- `CriticalPathComplexity`: 11
- `MaxBatchWidth`: 2
- `OverlapHotspots`: `crates/plan-issue-cli/src/execute.rs` tests overlap on `sync_issue_rows_from_task_spec`; keep exact-test additions isolated from contract-doc edits.
**Parallelizable tasks**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.3` depends on both to align wording and assertions.

### Task 1.1: Define Task Decomposition runtime-truth contract and owner semantics
- **Location**:
  - `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md`
  - `crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md`
  - `crates/plan-issue-cli/docs/README.md`
- **Description**: Update contract and runbook docs to define `Task Decomposition` as the only runtime-truth execution table, clarify `Owner` as a dispatch alias, document lane canonicalization for shared/per-sprint execution, and explicitly reject adding a second issue-body dispatch table.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Contract docs describe which columns are runtime-truth and which remain runtime-progress fields (`PR`, `Status`).
  - `group + auto` single-lane normalization behavior is documented with `per-sprint` semantics and canonical lane metadata requirements.
  - README/spec language no longer implies that task-spec or subagent prompts may intentionally differ from `Task Decomposition`.
- **Validation**:
  - `rg -n 'runtime-truth|Task Decomposition|dispatch alias|single-lane|per-sprint' crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md crates/plan-issue-cli/docs/README.md`

### Task 1.2: Add characterization tests for auto single-lane metadata drift
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/src/task_spec.rs`
- **Description**: Extend current unit tests to capture the existing mismatch where `group + auto` single-lane sprints get `Execution Mode=per-sprint` but retain per-task `Owner`/`Branch`/`Worktree` and non-canonical notes, and add a focused test matrix for the intended normalized lane invariants (initially failing/ignored as appropriate).
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Existing auto single-group test explicitly asserts current lane metadata mismatch (or is replaced by a stronger failing-target test with clear diagnostics).
  - `task_spec`-level tests cover single-lane auto/group and multi-group auto/group execution-mode classification and lane canonicalization expectations.
  - Tests identify which row should act as the canonical lane anchor (`shared-pr-anchor` or deterministic fallback).
- **Validation**:
  - `cargo test -p nils-plan-issue-cli sync_issue_rows_from_task_spec_auto_single_group_uses_per_sprint_mode -- --exact`
  - `cargo test -p nils-plan-issue-cli task_spec::tests::execution_mode_by_task_auto_single_lane_uses_per_sprint -- --exact`

### Task 1.3: Add end-to-end characterization coverage for source-of-truth drift between issue table and artifacts
- **Location**:
  - `crates/plan-issue-cli/tests/runtime_truth_plan_and_sprint_flow.rs`
  - `crates/plan-issue-cli/tests/live_start_sprint_runtime_truth.rs`
  - `crates/plan-issue-cli/tests/task_spec_flow.rs`
- **Description**: Add characterization coverage that records current `start-plan`/`start-sprint` behavior where task-spec rows are recomputed and used to overwrite issue rows, plus prompt/task-spec artifact outputs that can disagree with shared/per-sprint lane semantics.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests capture current overwrite path in `start-sprint` and document expected refactor target behavior.
  - Task-spec/prompt artifact assertions demonstrate the current per-task prompt generation mismatch for single-lane auto/group sprints.
  - Fixtures/tests are isolated enough to be safely updated in later sprints without broad shell-parity churn.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test task_spec_flow`

## Sprint 2: Canonical runtime lane metadata and row validation
**Goal**: Implement lane canonicalization so task rows carry actual runtime lane metadata, including `group + auto` single-lane `per-sprint` normalization, and enforce row-level invariants that prevent drift from passing preflight.
**PR grouping intent**: `group` (lane-core lane + sync/render lane + validation/tests lane).
**PR group mapping (deterministic validation)**:
- `S2T1=lane-core`
- `S2T2=lane-sync`
- `S2T3=lane-validate`
- `S2T4=lane-validate`
**Execution Profile**: `parallel-x2` (parallel width 2).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-plan-issue-cli sync_issue_rows_from_task_spec_auto_single_group_uses_per_sprint_mode -- --exact`
  - `cargo test -p nils-plan-issue-cli sync_issue_rows_from_task_spec_auto_multi_group_keeps_group_modes -- --exact`
  - `cargo test -p nils-plan-issue-cli linked_worktree_listing_and_cleanup_modes_are_covered -- --exact`
- Verify:
  - Single-lane `group + auto` rows use `per-sprint` plus canonical `Owner`/`Branch`/`Worktree` and normalized notes tokens.
  - `pr-shared` rows that truly share a lane use identical lane metadata across rows.
  - Row validation blocks inconsistent lane metadata for rows that claim shared/per-sprint execution.
**Sprint scorecard**:
- `TotalComplexity`: 19
- `CriticalPathComplexity`: 15
- `MaxBatchWidth`: 2
- `OverlapHotspots`: `crates/plan-issue-cli/src/task_spec.rs` and `crates/plan-issue-cli/src/execute.rs` share normalization logic and sync call sites; `issue_body.rs` validation can run in parallel but converges in integration tests.
**Parallelizable tasks**:
- `Task 2.2` and `Task 2.3` can run in parallel after `Task 2.1`.
- `Task 2.4` integrates the outputs and runs regression coverage.

### Task 2.1: Implement runtime lane canonicalization helper from task-spec rows
- **Location**:
  - `crates/plan-issue-cli/src/task_spec.rs`
  - `crates/plan-issue-cli/src/issue_body.rs`
  - `crates/plan-tooling/tests/split_prs.rs`
- **Description**: Add a deterministic lane-canonicalization helper that derives effective execution mode and canonical lane metadata per task row (using `pr_group`, sprint-level group counts, and `shared-pr-anchor` notes metadata), including the auto single-lane `per-sprint` normalization case, and add regression assertions that lock the required `split-prs` notes metadata keys used by the helper.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Helper returns canonical `Execution Mode` and lane metadata (`Owner`, `Branch`, `Worktree`) for every task row.
  - Auto/group single-lane sprints normalize to one per-sprint lane while preserving per-task rows.
  - Canonicalization remains deterministic across repeated runs on the same input rows.
  - Regression coverage protects the `pr-group` and `shared-pr-anchor` notes metadata expectations relied on by canonicalization.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli task_spec::tests::execution_mode_by_task_auto_single_lane_uses_per_sprint -- --exact`
  - `cargo test -p nils-plan-issue-cli task_spec::tests::runtime_lane_canonicalization_uses_shared_anchor_metadata -- --exact`
  - `cargo test -p nils-plan-tooling --test split_prs split_prs_non_regression_required_notes_keys -- --exact`

### Task 2.2: Apply canonical lane metadata in issue row sync and start-plan rendering paths
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/src/render.rs`
- **Description**: Refactor issue-row sync and issue-body initial render helpers to populate `Owner`/`Branch`/`Worktree`/`Execution Mode`/notes from canonical lane metadata instead of raw per-task split rows, eliminating the display/runtime mismatch before source-of-truth cutover.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `sync_issue_rows_from_task_spec` writes canonical lane metadata and canonicalized `Execution Mode`.
  - `start-plan` issue body generation no longer seeds runtime-lane columns with placeholder values when task-spec data is available.
  - Single-lane auto/group sprint rows render with identical lane metadata across all tasks.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli sync_issue_rows_from_task_spec_updates_table_and_detects_missing_rows -- --exact`
  - `cargo test -p nils-plan-issue-cli sync_issue_rows_from_task_spec_auto_single_group_uses_per_sprint_mode -- --exact`
  - `cargo test -p nils-plan-issue-cli render_issue_body_start_plan_writes_issue_body_artifact -- --exact`

### Task 2.3: Enforce runtime-truth lane invariants in Task Decomposition validation
- **Location**:
  - `crates/plan-issue-cli/src/issue_body.rs`
  - `crates/plan-issue-cli/src/execute.rs`
- **Description**: Strengthen `Task Decomposition` validation so rows that declare `per-sprint` or `pr-shared` enforce lane metadata consistency within the lane, reject contradictory owner/branch/worktree combinations, and keep owner alias checks aligned with runtime-truth semantics.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Validation errors identify conflicting lane metadata across rows that share a lane.
  - `per-sprint` rows within one sprint cannot pass with multiple lane metadata combinations.
  - Existing `pr-isolated` uniqueness checks remain intact and compatible with shared/per-sprint canonicalization.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli validate_rows_detects_conflicting_shared_lane_metadata -- --exact`
  - `cargo test -p nils-plan-issue-cli validate_rows_flags_non_subagent_owner_for_done_rows -- --exact`

### Task 2.4: Add regression coverage for cleanup targeting and normalized notes metadata
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/tests/live_start_sprint_runtime_truth.rs`
  - `crates/plan-issue-cli/tests/task_spec_flow.rs`
- **Description**: Add tests confirming cleanup and downstream row parsing still work when multiple rows share canonical lane metadata and notes are normalized for the single-lane auto/per-sprint path.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Cleanup targets the actual canonical branch/worktree lane without missing shared-lane rows.
  - Notes normalization preserves machine-parsed tokens (`sprint=`, `pr-group=`, optional `shared-pr-anchor=`) required by row parsing and traceability.
  - Regression tests cover both single-lane and multi-lane auto/group sprints.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli linked_worktree_listing_and_cleanup_modes_are_covered -- --exact`
  - `cargo test -p nils-plan-issue-cli --test task_spec_flow`

## Sprint 3: Issue creation precomputes runtime plan and sprint execution consumes Task Decomposition
**Goal**: Make issue creation populate the executable runtime plan and make sprint execution read that plan directly for dispatch artifacts, task-spec export, and lane-aware prompt generation.
**PR grouping intent**: `group` (start-plan/runtime-table lane + artifact-generation lane + start-sprint-cutover lane + integration-tests lane).
**PR group mapping (deterministic validation)**:
- `S3T1=issue-init`
- `S3T2=artifacts`
- `S3T3=start-sprint`
- `S3T4=integration`
**Execution Profile**: `parallel-x3` (parallel width 3).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`
  - `cargo test -p nils-plan-issue-cli --test task_spec_flow`
- Verify:
  - `start-plan` precomputes a full-plan executable dispatch layout and writes concrete runtime-truth metadata into `Task Decomposition`.
  - `start-sprint` dispatch artifacts are derived from issue rows, not fresh split rows, and drift becomes a hard error instead of silent overwrite.
  - Single-lane auto/group sprints produce per-sprint lane-aware prompts/artifacts that match issue rows exactly.
**Sprint scorecard**:
- `TotalComplexity`: 21
- `CriticalPathComplexity`: 16
- `MaxBatchWidth`: 2
- `OverlapHotspots`: `crates/plan-issue-cli/src/execute.rs` is touched by all runtime tasks; keep `Task 3.1` and `Task 3.3` merge order stable and isolate `Task 3.2` helper APIs for easier integration.
**Parallelizable tasks**:
- `Task 3.1` and `Task 3.2` can run in parallel after Sprint 2 lands.
- `Task 3.3` depends on both.
- `Task 3.4` runs after `Task 3.3` and updates end-to-end assertions.

### Task 3.1: Populate runtime-truth Task Decomposition during issue creation using full-plan split output
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/src/render.rs`
  - `crates/plan-issue-cli/src/task_spec.rs`
- **Description**: Refactor `start-plan` to build the full plan split once (including `--pr-grouping group --strategy auto` flows), validate/canonicalize the runtime lanes, and render `Task Decomposition` with executable lane metadata instead of placeholder lane-column values.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 5
- **Acceptance criteria**:
  - `start-plan` writes concrete `Owner`/`Branch`/`Worktree`/`Execution Mode`/notes for all task rows at issue creation time.
  - Row validation runs against the generated table before live GitHub writes and fails if runtime-truth invariants are violated.
  - `PR` and `Status` remain mutable workflow fields while execution metadata remains stable unless an explicit refresh workflow is invoked.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow start_plan_dry_run_writes_runtime_truth_task_decomposition_metadata -- --exact`
  - `cargo test -p nils-plan-issue-cli --test output_contract`

### Task 3.2: Generate task-spec and subagent prompts from Task Decomposition rows with lane-aware prompt deduplication
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/src/issue_body.rs`
  - `crates/plan-issue-cli/src/task_spec.rs`
- **Description**: Add adapters that derive task-spec rows and lane-aware subagent prompts directly from parsed `Task Decomposition` rows, including one prompt per actual execution lane (with task lists) when rows share a per-sprint/pr-shared lane.
- **Dependencies**:
  - Task 2.1
  - Task 2.3
- **Complexity**: 5
- **Acceptance criteria**:
  - Task-spec export can be generated from issue rows without recomputing split assignments.
  - Prompt generation deduplicates by canonical lane and includes all tasks assigned to the lane.
  - Artifact contents (`Owner`/`Branch`/`Worktree`/`Execution Mode`/task list) match the issue table exactly.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli write_subagent_prompts_groups_tasks_by_runtime_lane -- --exact`
  - `cargo test -p nils-plan-issue-cli task_spec_from_issue_rows_preserves_runtime_truth_metadata -- --exact`

### Task 3.3: Refactor start-sprint to dispatch from issue table and fail on drift instead of rewriting from fresh split rows
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/src/github.rs`
  - `crates/plan-issue-cli/tests/runtime_truth_plan_and_sprint_flow.rs`
- **Description**: Change `start-sprint` so it parses and validates `Task Decomposition`, selects rows for the target sprint, derives runtime artifacts from those rows, and treats mismatches against current plan-derived split output (if checked) as explicit drift errors rather than silently overwriting the issue table.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Sprint kickoff artifacts and comments are generated from issue-table rows.
  - `start-sprint` no longer overwrites runtime-lane columns from freshly computed split rows in normal operation.
  - Drift diagnostics are actionable (identify row/task/column mismatch and remediation path).
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow start_sprint_uses_issue_table_runtime_truth_and_rejects_drift -- --exact`
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`

### Task 3.4: Add end-to-end regression coverage for auto single-lane per-sprint runtime execution path
- **Location**:
  - `crates/plan-issue-cli/tests/runtime_truth_plan_and_sprint_flow.rs`
  - `crates/plan-issue-cli/tests/auto_single_lane_runtime_truth.rs`
  - `crates/plan-issue-cli/tests/fixtures/shell_parity`
- **Description**: Add full-flow regression tests/fixtures demonstrating that `group + auto` single-lane sprints are rendered, synced, and executed as `per-sprint` with one canonical runtime lane and consistent artifact outputs.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 5
- **Acceptance criteria**:
  - End-to-end tests cover `start-plan -> start-sprint -> ready-sprint` for a single-lane auto/group sprint.
  - Fixtures assert no contradictory lane metadata across issue rows and generated prompts/task-spec.
  - Multi-group auto behavior remains unchanged except for canonicalized shared-lane metadata.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`

## Sprint 4: Contract and fixture parity finalization
**Goal**: Finalize runtime-truth contracts/help/fixtures and stabilize regression outputs before the final documentation review sprint.
**PR grouping intent**: `per-sprint` (single integration PR to keep contract, fixtures, and verification synchronized).
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/plan-issue-cli-task-decomposition-runtime-truth-plan.md`
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test live_start_sprint_runtime_truth`
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`
- Verify:
  - Contracts/specs/help and fixtures describe runtime-truth semantics consistently before the final docs sweep.
  - Regression fixtures stabilize with canonical lane metadata and lane-aware prompt outputs.
  - Final release-gate checks remain deferred to Sprint 5 after README/docs review.
**Sprint scorecard**:
- `TotalComplexity`: 9
- `CriticalPathComplexity`: 9
- `MaxBatchWidth`: 1
- `OverlapHotspots`: contract/spec/help text and parity fixtures are intentionally serialized to avoid fixture drift while wording is still changing.
**Parallelizable tasks**:
- none (intentional serial integration gate).

### Task 4.1: Update contracts, runbooks, and help text for runtime-truth issue-table semantics
- **Location**:
  - `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md`
  - `crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md`
  - `crates/plan-issue-cli/docs/README.md`
  - `crates/plan-issue-cli/src/usage.rs`
- **Description**: Finalize user-facing and contributor-facing documentation so command help, contracts, and runbooks all describe issue-creation precompute + issue-table runtime-truth execution semantics (including auto single-lane per-sprint behavior and lane-aware prompt generation).
- **Dependencies**:
  - Task 3.4
- **Complexity**: 4
- **Acceptance criteria**:
  - Docs consistently describe `Task Decomposition` as runtime truth.
  - Help text and docs explain how drift is detected and how to refresh/recreate execution metadata safely.
  - No stale wording claims that runtime metadata is intentionally left as placeholders until sprint start.
- **Validation**:
  - `rg -n 'runtime truth|Task Decomposition|placeholder|start-sprint|lane-aware' crates/plan-issue-cli/docs crates/plan-issue-cli/src/usage.rs`

### Task 4.2: Refresh shell-parity and regression fixtures for runtime-truth outputs
- **Location**:
  - `crates/plan-issue-cli/tests/fixtures/shell_parity`
  - `crates/plan-issue-cli/tests/runtime_truth_plan_and_sprint_flow.rs`
  - `crates/plan-issue-cli/tests/live_start_sprint_runtime_truth.rs`
  - `crates/plan-issue-cli/tests/auto_single_lane_runtime_truth.rs`
- **Description**: Update parity and regression fixtures to match the new runtime-truth issue rows, lane-aware prompts, and drift diagnostics without changing unrelated user-visible behavior.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Fixture outputs reflect canonical lane metadata and lane-aware prompt behavior.
  - Regression tests remain deterministic after normalization changes.
  - Unrelated command outputs do not regress.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test live_start_sprint_runtime_truth`
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`

## Sprint 5: Final README/docs review and release gates
**Goal**: Perform a final documentation QA/review pass (README + specs + help text) after runtime behavior stabilizes, apply only residual corrections, then run the required release gates and rollback rehearsal.
**PR grouping intent**: `per-sprint` (single integration PR to keep final docs and release verification in lockstep).
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/plan-issue-cli-task-decomposition-runtime-truth-plan.md`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
- Verify:
  - `README.md`, crate READMEs, specs, and help text all describe the final runtime-truth and auto single-lane per-sprint behavior accurately.
  - Required checks and coverage gate pass after the docs/fixture review.
  - Rollback instructions are documented and operationally plausible.
**Sprint scorecard**:
- `TotalComplexity`: 12
- `CriticalPathComplexity`: 12
- `MaxBatchWidth`: 1
- `OverlapHotspots`: final docs review and release checks are intentionally serialized to avoid documenting behavior that has not been verified.
**Parallelizable tasks**:
- none (intentional serial final integration gate).

### Task 5.1: Final review and update README/docs/help surfaces for released runtime-truth behavior
- **Location**:
  - `README.md`
  - `crates/plan-issue-cli/README.md`
  - `crates/plan-tooling/README.md`
  - `crates/plan-issue-cli/docs/README.md`
  - `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md`
  - `crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md`
  - `crates/plan-issue-cli/src/usage.rs`
- **Description**: Run a final doc-review sprint that audits README/help/spec surfaces against the already-stabilized runtime behavior and applies only residual corrections needed to match shipped runtime-truth `Task Decomposition` behavior, lane-aware prompt generation, and `group + auto` single-lane `per-sprint` execution semantics.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Root and crate READMEs no longer describe the old display-vs-runtime split, with changes limited to residual corrections after Sprint 4.
  - Sprint 5 documentation edits are limited to final QA corrections and do not reopen already-stabilized runtime semantics or fixture decisions from Sprint 4.
  - Docs/examples mention the single-lane auto/group `per-sprint` behavior and canonical lane metadata expectations without reopening settled runtime semantics.
  - Help text and docs use consistent terminology for runtime-truth rows, drift detection, and refresh/recreate workflows.
- **Validation**:
  - `rg -n 'Task Decomposition|runtime truth|per-sprint|pr-shared|lane-aware|task-spec' README.md crates/plan-issue-cli/README.md crates/plan-tooling/README.md crates/plan-issue-cli/docs/README.md crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md crates/plan-issue-cli/src/usage.rs`

### Task 5.2: Run required checks, coverage gate, and rollback rehearsal for the cutover
- **Location**:
  - `DEVELOPMENT.md`
  - `README.md`
  - `crates/plan-issue-cli/docs/README.md`
  - `docs/plans/plan-issue-cli-task-decomposition-runtime-truth-plan.md`
- **Description**: Execute required lint/test checks and workspace coverage, then rehearse a rollback path (revert/refreeze to current task-spec-as-source behavior) so the refactor can be delivered safely.
- **Dependencies**:
  - Task 5.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Required checks pass or failures are documented with remediation before merge.
  - Coverage remains at or above the workspace threshold (`>=85%`).
  - Rollback rehearsal explicitly follows and validates the steps in this plan's `Rollback plan` section so current behavior can be restored if the runtime-truth cutover regresses.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - `rg -n '^## Rollback plan|^1\\.|^2\\.|^3\\.|^4\\.' docs/plans/plan-issue-cli-task-decomposition-runtime-truth-plan.md`

## Testing Strategy
- Unit:
  - Canonical lane normalization helpers (`execution_mode`, anchor resolution, metadata normalization, lane consistency checks).
  - `Task Decomposition` row validation invariants for `per-sprint`, `pr-shared`, and `pr-isolated`.
- Integration:
  - `start-plan`, `start-sprint`, `ready-sprint`, and cleanup flows in `crates/plan-issue-cli/tests/*_delivery.rs`.
  - Task-spec and prompt artifact generation from issue rows (`task_spec_flow` and new lane-aware prompt tests).
- E2E/manual:
  - Dry-run `start-plan` + `start-sprint` on a representative multi-sprint plan using `--pr-grouping group --strategy auto`.
  - Manual verification that single-lane auto/group sprints produce one runtime lane (canonical metadata + lane-aware prompt) while task rows remain distinct.

## Risks & gotchas
- Converting issue rows into the sole runtime source tightens validation; existing manually edited issues may fail until repaired.
- Lane-aware prompt deduplication is a contract change and may require fixture/help/runbook updates if downstream scripts assume one prompt file per task.
- Notes token normalization must preserve machine-parsed tokens (`sprint=`, `pr-group=`) or sprint gating and traceability can regress.
- Drift detection policy must be explicit to avoid confusing users when the plan file changes after issue creation.

## Rollback plan
1. Keep the refactor staged in small PRs so runtime-lane canonicalization and source-of-truth cutover can be reverted independently.
2. If source-of-truth cutover regresses sprint execution, revert the `start-sprint` issue-row dispatch path first and temporarily restore task-spec-as-source behavior while keeping non-breaking validation improvements.
3. If canonical lane normalization regresses cleanup or row validation, revert the normalization helper/sync integration commits and restore current per-task lane metadata behavior.
4. Re-run `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`, `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`, and required checks before reattempting the cutover.
