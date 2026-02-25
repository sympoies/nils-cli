# Plan: split-prs runtime metadata decoupling for plan-issue

## Overview
This plan removes `branch`, `worktree`, `owner`, and `notes` from `plan-tooling split-prs` output, then makes `plan-issue` fully materialize runtime execution metadata from grouping results plus parsed plan content. The runtime source-of-truth remains `Task Decomposition` in issue body, but metadata generation authority moves to `plan-issue` lane logic instead of split-prs task-level placeholders. The rollout is strictly linear (serial sprints, serial tasks) to reduce regression risk during broad contract/test rewrites.

## Scope
- In scope:
  - `crates/plan-tooling` split-prs output schema and contract/docs/fixtures.
  - `crates/plan-issue-cli` task-spec generation, lane materialization, issue-body rendering, drift checks, and cleanup compatibility.
  - `crates/nils-common` shared markdown helpers for payload validation and markdown-table-safe canonicalization.
  - Runtime notes canonicalization rules (including markdown-table-safe normalization) centralized in shared helpers.
  - Full test-suite rewrites impacted by removed split-prs fields and updated runtime-truth flow.
- Out of scope:
  - Reworking plan markdown parser format (`plan-tooling parse` input contract stays the same).
  - Changing `Task Decomposition` table columns in issue body.
  - Introducing cross-sprint execution parallelism.

## Assumptions
1. `split-prs` still emits deterministic grouping primitives needed by orchestrators (`task_id`, `summary`, `pr_group`, scope metadata).
2. `plan-issue` can derive runtime metadata (`owner`, `branch`, `worktree`, `notes`) from plan structure + grouping + prefixes without consuming removed split-prs fields.
3. `notes` remains required in runtime-truth issue rows because `pr-group` and `shared-pr-anchor` are currently encoded there.
4. All sprints are integration gates executed in order; no sprint-level parallel execution is allowed.
5. Existing `nils-common::markdown::validate_markdown_payload` remains the base guard for escaped control artifacts and will be extended (not replaced) for table-cell canonicalization.

## Success criteria
- `split-prs` JSON/TSV output no longer includes `branch`, `worktree`, `owner`, `notes`.
- `plan-issue` commands (`start-plan`, `start-sprint`, `ready-sprint`, `accept-sprint`) run without relying on removed split-prs fields.
- Runtime lane metadata becomes deterministic and directly executable from `plan-issue` materialization logic.
- Markdown-table-sensitive notes canonicalization is implemented via shared `nils-common` helpers and used consistently in render + compare paths.
- `Task Decomposition` drift checks and `cleanup-worktrees` remain stable under the new data flow.
- Test fixtures and contract docs are fully updated to the new schema and runtime behavior.
- All touched crate docs and root README crate sections are reconciled with final runtime ownership semantics.

## Sprint 1: Define and cut over split-prs reduced output schema
**Goal**: Replace split-prs task-level runtime placeholder fields with a reduced grouping-focused output contract.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/plan-split-prs-runtime-metadata-decoupling-plan.md`
  - `cargo test -p nils-plan-tooling --test split_prs`
  - `cargo test -p nils-plan-tooling split_prs`
- Verify:
  - split-prs header/schema no longer includes removed fields.
  - deterministic ordering and grouping behavior remain unchanged.
  - docs/specs/fixtures match the new schema exactly.
**Sprint scorecard**:
- `TotalComplexity`: 14
- `CriticalPathComplexity`: 14
- `MaxBatchWidth`: 1
- `OverlapHotspots`: `crates/plan-tooling/src/split_prs.rs` and split_prs fixtures/tests are edited in one serialized path.
**Parallelizable tasks**:
- none (intentional serial sequencing).

### Task 1.1: Update split-prs contract/spec and migration docs for reduced schema
- **Location**:
  - `crates/plan-tooling/docs/specs/split-prs-contract-v2.md`
  - `crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
  - `crates/plan-tooling/README.md`
  - `crates/plan-tooling/docs/runbooks/split-prs-migration.md`
  - `crates/plan-tooling/docs/runbooks/split-prs-build-task-spec-cutover.md`
- **Description**: Rewrite split-prs output contract sections to remove `branch/worktree/owner/notes`, document that runtime execution metadata is materialized by plan-issue lane logic, and add migration guidance for downstream consumers.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Spec/output examples no longer reference removed fields.
  - New contract examples are published under v2 while v1 is retained as legacy reference.
  - Migration docs clearly map old fields to new plan-issue materialization path.
  - No stale schema snippets remain.
- **Validation**:
  - `test -z "$(rg -n '\\b(branch|worktree|owner|notes)\\b' crates/plan-tooling/docs/specs/split-prs-contract-v2.md || true)"`
  - `rg -n 'v2|legacy|plan-issue.*materializ' crates/plan-tooling/docs/runbooks/split-prs-migration.md`
  - `rg -n 'task_id.*summary.*pr_group' crates/plan-tooling/README.md`

### Task 1.2: Refactor split-prs structs and renderers to emit reduced records
- **Location**:
  - `crates/plan-tooling/src/split_prs.rs`
  - `crates/plan-tooling/src/lib.rs`
- **Description**: Change CLI-facing `OutputRecord` and JSON/TSV renderers to keep only grouping-relevant fields (`task_id`, `summary`, `pr_group`) while preserving deterministic scope/strategy behavior and explain payload semantics. Keep runtime metadata inside internal split record structures as a temporary compatibility bridge until Sprint 2 plan-issue cutover is complete.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - split-prs CLI JSON/TSV output shape matches new contract.
  - grouping behavior (`deterministic|auto`, `per-sprint|group`) remains deterministic.
  - no runtime metadata field is emitted by split-prs.
- **Validation**:
  - `cargo test -p nils-plan-tooling --test split_prs`
  - `cargo test -p nils-plan-tooling split_prs`

### Task 1.3: Rewrite split-prs fixtures and schema assertions
- **Location**:
  - `crates/plan-tooling/tests/split_prs.rs`
  - `crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.tsv`
  - `crates/plan-tooling/tests/fixtures/split_prs/group_expected.tsv`
  - `crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.json`
  - `crates/plan-tooling/tests/fixtures/split_prs/group_expected.json`
- **Description**: Update fixture files and test assertions to the reduced output schema while preserving deterministic grouping and anchor semantics.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - fixture headers and JSON keys match reduced schema.
  - split_prs tests pass without compatibility shims.
  - anchor and group determinism checks stay intact.
- **Validation**:
  - `cargo test -p nils-plan-tooling --test split_prs`
  - `rg -n '^# task_id\\tsummary\\tpr_group$' crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.tsv crates/plan-tooling/tests/fixtures/split_prs/group_expected.tsv`

## Sprint 2: Build plan-issue runtime metadata materializer independent from split-prs removed fields
**Goal**: Make plan-issue generate executable lane metadata (`branch/worktree/owner/notes`) from plan + grouping primitives, with markdown-table canonicalization centralized in `nils-common`.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-common markdown`
  - `cargo test -p nils-plan-issue-cli task_spec`
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`
- Verify:
  - build_task_spec no longer reads removed split-prs fields.
  - runtime metadata is synthesized deterministically.
  - markdown table canonicalization is shared and applied before render/compare boundaries.
  - single-lane auto groups still materialize as executable per-sprint lanes.
**Sprint scorecard**:
- `TotalComplexity`: 16
- `CriticalPathComplexity`: 16
- `MaxBatchWidth`: 1
- `OverlapHotspots`: `crates/plan-issue-cli/src/task_spec.rs`, `crates/plan-issue-cli/src/execute.rs`, and `crates/nils-common/src/markdown.rs` are serialized to prevent dual-source metadata bugs.
**Parallelizable tasks**:
- none (intentional serial sequencing).

### Task 2.1: Introduce structured runtime-metadata builder API in plan-issue
- **Location**:
  - `crates/plan-issue-cli/src/task_spec.rs`
  - `crates/plan-issue-cli/src/lib.rs`
  - `crates/plan-issue-cli/src/commands/mod.rs`
- **Description**: Add a dedicated builder that consumes parsed plan tasks, split grouping results, strategy, and prefixes to produce deterministic runtime `owner/branch/worktree/notes` per lane.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - metadata generation has a single code path used by build/start/ready/accept flows.
  - generated metadata does not depend on split-prs removed fields.
  - deterministic output is stable across repeated runs.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli task_spec`

### Task 2.2: Define deterministic anchor and notes synthesis rules without split-prs notes input
- **Location**:
  - `crates/plan-issue-cli/src/task_spec.rs`
  - `crates/plan-issue-cli/src/issue_body.rs`
  - `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md`
- **Description**: Compute anchor from lane membership (`sprint + pr_group + stable task ordering`) and synthesize canonical notes tokens (`sprint`, `plan-task`, `deps`, `validate`, `pr-grouping`, `pr-group`, `shared-pr-anchor`) directly from plan/task metadata.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - notes always include `pr-group` for runtime parsing compatibility.
  - shared lanes always emit deterministic `shared-pr-anchor`.
  - no reliance on passthrough free-form notes from split-prs.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test task_spec_flow`

### Task 2.3: Centralize markdown-table-safe notes canonicalization in nils-common and adopt in plan-issue
- **Location**:
  - `crates/nils-common/src/markdown.rs`
  - `crates/nils-common/src/lib.rs`
  - `crates/nils-common/tests/markdown_table_canonicalization.rs`
  - `crates/plan-issue-cli/src/task_spec.rs`
  - `crates/plan-issue-cli/src/issue_body.rs`
  - `crates/plan-issue-cli/src/execute.rs`
- **Description**: Add a shared markdown helper API in `nils-common::markdown` (for example `canonicalize_table_cell`) that normalizes `|`, `\n`, and `\r` for markdown-table-safe round-trips; then replace plan-issue local ad-hoc sanitization with the shared helper at generation, render, and comparison boundaries.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - canonicalization logic has exactly one production implementation in `nils-common`.
  - plan-issue code paths no longer duplicate markdown table sanitization logic.
  - start-plan generated rows do not drift solely due to markdown sanitization.
  - start-sprint drift checks only flag semantic mismatches.
  - notes parity is stable after parse/render cycles (idempotent canonicalization).
- **Validation**:
  - `cargo test -p nils-common markdown`
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`
  - `cargo test -p nils-plan-issue-cli --test live_issue_ops`

## Sprint 3: Integrate runtime materialization across orchestration flows and guardrails
**Goal**: Ensure every plan-issue flow uses the new materialized runtime metadata consistently.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test live_start_sprint_runtime_truth`
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`
- Verify:
  - start/ready/accept flows render and validate rows from one runtime metadata source.
  - drift and cleanup checks remain deterministic.
  - subagent prompt lane grouping remains correct.
**Sprint scorecard**:
- `TotalComplexity`: 14
- `CriticalPathComplexity`: 14
- `MaxBatchWidth`: 1
- `OverlapHotspots`: execution-path functions in `execute.rs` and `render.rs` are serialized to avoid temporary mixed semantics.
**Parallelizable tasks**:
- none (intentional serial sequencing).

### Task 3.1: Update start-plan rendering to use plan-issue materialized runtime metadata only
- **Location**:
  - `crates/plan-issue-cli/src/render.rs`
  - `crates/plan-issue-cli/src/execute.rs`
- **Description**: Remove assumptions that split rows already carry executable metadata and render Task Decomposition from materializer output.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 5
- **Acceptance criteria**:
  - start-plan issue body rows are executable without post-hoc lane patching.
  - per-sprint and group flows produce deterministic runtime rows.
  - local/live output contracts remain stable.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`

### Task 3.2: Rewire start-sprint and drift checks to compare canonical runtime metadata
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/src/task_spec.rs`
- **Description**: Update runtime plan comparison and sync paths to use canonicalized metadata generated by plan-issue, removing any implicit dependency on split-prs removed columns.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - drift checks compare canonical lane metadata and fail only on real mismatches.
  - start-sprint artifact generation stays aligned with issue-table runtime truth.
  - no split-prs removed-field access remains.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test live_start_sprint_runtime_truth`
  - `cargo test -p nils-plan-issue-cli --test task_spec_flow`

### Task 3.3: Validate cleanup-worktrees and lane prompts under new metadata source
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/tests/live_issue_ops.rs`
  - `crates/plan-issue-cli/tests/live_start_sprint_runtime_truth.rs`
- **Description**: Confirm cleanup target resolution and prompt lane aggregation still derive correct `branch/worktree/owner` from materialized runtime metadata.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 4
- **Acceptance criteria**:
  - cleanup targets resolve correctly from Task Decomposition rows.
  - one prompt per runtime lane remains deterministic.
  - no regression in gate behavior around merged PR checks.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test live_issue_ops`
  - `cargo test -p nils-plan-issue-cli --test live_start_sprint_runtime_truth`

## Sprint 4: Rewrite tests, fixtures, and docs for new end-to-end model
**Goal**: Replace old assumptions (split-prs carries runtime metadata) with new runtime-materialization assertions.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-plan-tooling --test split_prs`
  - `cargo test -p nils-plan-issue-cli`
  - `bash scripts/ci/docs-placement-audit.sh --strict`
- Verify:
  - split-prs tests validate reduced output schema.
  - plan-issue tests assert runtime metadata is generated locally.
  - docs/specs/runbooks match runtime behavior.
**Sprint scorecard**:
- `TotalComplexity`: 10
- `CriticalPathComplexity`: 10
- `MaxBatchWidth`: 1
- `OverlapHotspots`: test rewrites in `sprint3/4/5_delivery` and `task_spec_flow/live_issue_ops` are serialized to avoid expectation conflicts.
**Parallelizable tasks**:
- none (intentional serial sequencing).

### Task 4.1: Rewrite plan-issue integration tests for runtime materialization-first behavior
- **Location**:
  - `crates/plan-issue-cli/tests/runtime_truth_plan_and_sprint_flow.rs`
  - `crates/plan-issue-cli/tests/live_start_sprint_runtime_truth.rs`
  - `crates/plan-issue-cli/tests/auto_single_lane_runtime_truth.rs`
  - `crates/plan-issue-cli/tests/task_spec_flow.rs`
  - `crates/plan-issue-cli/tests/live_issue_ops.rs`
- **Description**: Remove assertions that depend on pre-convergence split rows (for example anchor/non-anchor `assert_ne` expectations) and replace them with assertions on deterministic materialized runtime rows.
- **Dependencies**:
  - Task 3.3
- **Complexity**: 6
- **Acceptance criteria**:
  - runtime-truth tests assert one canonical lane metadata set per lane.
  - start-plan/start-sprint round-trip tests no longer rely on removed split fields.
  - drift tests still detect manual table edits.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow`
  - `cargo test -p nils-plan-issue-cli --test auto_single_lane_runtime_truth`

### Task 4.2: Align user-facing docs and contracts with new data-flow ownership
- **Location**:
  - `crates/plan-issue-cli/README.md`
  - `crates/plan-issue-cli/docs/README.md`
  - `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md`
  - `crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md`
  - `docs/plans/plan-issue-cli-tooling-delivery-loop-alignment-plan.md`
- **Description**: Document that split-prs outputs grouping primitives only, while plan-issue owns runtime lane metadata materialization and issue-table execution truth.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 4
- **Acceptance criteria**:
  - no docs imply split-prs still ships runtime execution metadata fields.
  - runtime-truth flow is consistent across README/specs/runbooks.
  - migration caveats for downstream consumers are explicit.
- **Validation**:
  - `test -z "$(rg -n 'split-prs.*(branch|worktree|owner|notes)' crates/plan-issue-cli/README.md crates/plan-issue-cli/docs crates/plan-tooling/docs || true)"`

## Sprint 5: Gate validation, migration safety checks, and release readiness
**Goal**: Prove the linear migration is stable under required checks and produce rollback-ready artifacts.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Verify:
  - required lint/test/completion gates pass.
  - workspace coverage gate stays `>= 85`.
  - migration docs + smoke commands are reproducible.
**Sprint scorecard**:
- `TotalComplexity`: 10
- `CriticalPathComplexity`: 10
- `MaxBatchWidth`: 1
- `OverlapHotspots`: required-check script and coverage artifacts are run sequentially to keep failure diagnosis linear.
**Parallelizable tasks**:
- none (intentional serial sequencing).

### Task 5.1: Execute required workspace verification gates after migration
- **Location**:
  - `DEVELOPMENT.md`
  - `scripts/ci/docs-placement-audit.sh`
  - `scripts/ci/coverage-summary.sh`
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- **Description**: Run the mandatory repo checks end-to-end after code/docs/test migration and collect failures with remediation notes.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 3
- **Acceptance criteria**:
  - required checks pass or failures are clearly triaged.
  - no stale assumptions remain in completion/docs placement checks.
  - outputs are reproducible from clean workspace state.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

### Task 5.2: Re-run coverage gate and capture migration evidence artifacts
- **Location**:
  - `target/coverage/lcov.info`
  - `scripts/ci/coverage-summary.sh`
  - `$AGENT_HOME/out/plan-split-prs-runtime-metadata-decoupling/migration-evidence.txt`
- **Description**: Confirm workspace coverage gate after test rewrites and store concise migration evidence (schema diffs, command outputs) for reviewer signoff.
- **Dependencies**:
  - Task 5.1
- **Complexity**: 4
- **Acceptance criteria**:
  - coverage gate remains above threshold.
  - evidence artifacts are produced with deterministic paths.
  - reviewers can replay key smoke commands from documentation.
- **Validation**:
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

### Task 5.3: Final migration smoke checklist for split-prs and plan-issue command flow
- **Location**:
  - `crates/plan-tooling/docs/runbooks/split-prs-migration.md`
  - `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v2.md`
  - `crates/plan-issue-cli/tests/fixtures/shell_parity/multi_sprint_guide_dry_run.txt`
- **Description**: Add and verify a final smoke checklist that exercises reduced split-prs output plus plan-issue runtime materialization path (start-plan/start-sprint loop) and deterministic split reproducibility checks.
- **Dependencies**:
  - Task 5.2
- **Complexity**: 3
- **Acceptance criteria**:
  - smoke checklist commands run successfully in dry-run mode.
  - checklist reflects the new schema and runtime ownership model.
  - deterministic split output checks (`scope=plan` + rerun diff) are documented and reproducible.
- **Validation**:
  - `mkdir -p "$AGENT_HOME/out/plan-split-prs-runtime-metadata-decoupling"`
  - `cargo run -p nils-plan-tooling -- split-prs --file docs/plans/plan-split-prs-runtime-metadata-decoupling-plan.md --scope plan --pr-grouping per-sprint --strategy deterministic --format json > "$AGENT_HOME/out/plan-split-prs-runtime-metadata-decoupling/split-prs-plan-a.json"`
  - `cargo run -p nils-plan-tooling -- split-prs --file docs/plans/plan-split-prs-runtime-metadata-decoupling-plan.md --scope plan --pr-grouping per-sprint --strategy deterministic --format json > "$AGENT_HOME/out/plan-split-prs-runtime-metadata-decoupling/split-prs-plan-b.json"`
  - `diff -u "$AGENT_HOME/out/plan-split-prs-runtime-metadata-decoupling/split-prs-plan-a.json" "$AGENT_HOME/out/plan-split-prs-runtime-metadata-decoupling/split-prs-plan-b.json"`
  - `cargo run -p nils-plan-tooling -- split-prs --file docs/plans/plan-split-prs-runtime-metadata-decoupling-plan.md --scope sprint --sprint 1 --pr-grouping group --strategy deterministic --pr-group 'Task 1.1=g1' --pr-group 'Task 1.2=g1' --pr-group 'Task 1.3=g2' --format json`
  - `cargo run -p nils-plan-tooling -- split-prs --file crates/plan-tooling/tests/fixtures/split_prs/auto_overlap_plan.md --scope sprint --sprint 1 --pr-grouping group --strategy auto --format json`
  - `cargo run -p nils-plan-issue-cli --bin plan-issue-local -- start-plan --plan docs/plans/plan-split-prs-runtime-metadata-decoupling-plan.md --pr-grouping per-sprint --dry-run`
  - `cargo run -p nils-plan-issue-cli --bin plan-issue-local -- start-sprint --plan docs/plans/plan-split-prs-runtime-metadata-decoupling-plan.md --issue 999 --sprint 1 --pr-grouping per-sprint --no-comment --dry-run`
  - `cargo run -p nils-plan-issue-cli --bin plan-issue-local -- multi-sprint-guide --plan docs/plans/plan-split-prs-runtime-metadata-decoupling-plan.md --from-sprint 1 --to-sprint 2`

## Sprint 6: Documentation convergence for touched crates and workspace root
**Goal**: Reconcile all touched crate docs and root README crate content with the final split-prs/plan-issue runtime ownership and shared markdown canonicalization model.
**PR grouping intent**: `per-sprint`.
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
- Verify:
  - `plan-tooling`, `plan-issue-cli`, and `nils-common` docs align on data-flow ownership and shared markdown canonicalization semantics.
  - root `README.md` crate descriptions match the updated responsibilities.
  - no stale examples mention removed split-prs runtime metadata fields.
**Sprint scorecard**:
- `TotalComplexity`: 9
- `CriticalPathComplexity`: 9
- `MaxBatchWidth`: 1
- `OverlapHotspots`: crate READMEs/specs and root README are updated in one linear pass to avoid contradictory wording.
**Parallelizable tasks**:
- none (intentional serial sequencing).

### Task 6.1: Update nils-common docs for markdown canonicalization contract
- **Location**:
  - `crates/nils-common/README.md`
  - `crates/nils-common/docs/README.md`
  - `crates/nils-common/docs/specs/markdown-helpers-contract-v1.md`
- **Description**: Document the shared markdown helper contract (`validate_markdown_payload` + table-cell canonicalization helper), including intended call sites and idempotence expectations.
- **Dependencies**:
  - Task 5.3
- **Complexity**: 3
- **Acceptance criteria**:
  - docs explicitly describe canonicalization behavior for `|`, newline, and carriage return handling.
  - contract examples include before/after canonicalization samples.
  - no ambiguity remains on which crate owns this logic.
- **Validation**:
  - `rg -n 'canonicaliz|table.*cell|validate_markdown_payload' crates/nils-common/README.md crates/nils-common/docs/README.md crates/nils-common/docs/specs/markdown-helpers-contract-v1.md`

### Task 6.2: Reconcile plan-tooling and plan-issue docs with finalized runtime ownership
- **Location**:
  - `crates/plan-tooling/README.md`
  - `crates/plan-tooling/docs/specs/split-prs-contract-v2.md`
  - `crates/plan-tooling/docs/runbooks/split-prs-migration.md`
  - `crates/plan-issue-cli/README.md`
  - `crates/plan-issue-cli/docs/README.md`
  - `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v2.md`
- **Description**: Perform a final cross-check pass so split-prs docs consistently expose grouping-only outputs and plan-issue docs consistently define runtime metadata materialization and shared markdown canonicalization dependency.
- **Dependencies**:
  - Task 6.1
- **Complexity**: 3
- **Acceptance criteria**:
  - no conflicting wording across tooling/issue docs about ownership of `branch/worktree/owner/notes`.
  - split-prs examples only contain reduced fields.
  - plan-issue docs clearly point to `nils-common` for markdown canonicalization.
- **Validation**:
  - `test -z "$(rg -n 'split-prs.*(branch|worktree|owner|notes)' crates/plan-tooling/README.md crates/plan-tooling/docs/specs/split-prs-contract-v2.md crates/plan-tooling/docs/runbooks/split-prs-migration.md crates/plan-issue-cli/README.md crates/plan-issue-cli/docs || true)"`

### Task 6.3: Update workspace root README crate matrix and run final docs gate
- **Location**:
  - `README.md`
  - `DEVELOPMENT.md`
  - `docs/specs/crate-docs-placement-policy.md`
- **Description**: Align root crate descriptions with the new responsibilities (`plan-tooling` reduced output; `plan-issue-cli` runtime materialization; `nils-common` shared markdown canonicalization) and run final docs gate checks.
- **Dependencies**:
  - Task 6.2
- **Complexity**: 3
- **Acceptance criteria**:
  - root README crate section reflects updated crate roles and no stale field-level claims.
  - docs placement policy expectations remain satisfied after all doc moves/edits.
  - docs-only required checks pass.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`

## Testing Strategy
- Unit:
  - split-prs reduced output schema serialization/deserialization and deterministic grouping.
  - nils-common markdown helpers (`validate_markdown_payload` + table-cell canonicalization idempotence).
  - plan-issue runtime materializer (anchor selection, notes synthesis, shared-helper normalization).
- Integration:
  - start-plan/start-sprint/ready-sprint/accept-sprint with runtime-truth lane metadata generation.
  - drift detection and cleanup-worktrees behavior with canonicalized metadata.
- E2E/manual:
  - dry-run multi-sprint guide rehearsal on migrated plan.
  - full required-check and coverage gates from `DEVELOPMENT.md`.

## Risks & gotchas
- Hidden downstream parsers may still consume removed split-prs columns; migration docs and smoke checks must surface this early.
- If shared canonicalization helper and plan-issue adoption points diverge, false-positive drift failures will block sprints.
- Anchor selection must stay deterministic under both `deterministic` and `auto` grouping to avoid prompt/worktree churn.
- Markdown table sanitization (`|` conversion) can silently mutate notes if canonicalization is not applied before compare/render.
- Test rewrite volume is high; stale fixture assumptions can mask real regressions unless full suite is run after each sprint gate.

## Rollback plan
1. Revert split-prs schema reduction commits first and restore previous fixtures if downstream breakage appears before plan-issue migration lands.
2. If plan-issue materialization introduces runtime drift, temporarily pin to previous task-spec generation path while keeping new docs behind follow-up PR.
3. Revert notes canonicalization changes independently if they affect existing issue-body parsing compatibility.
4. Re-run `cargo test -p nils-plan-tooling --test split_prs`, `cargo test -p nils-plan-issue-cli`, and required checks after each rollback step before retrying.
