# Plan: plan-issue-cli / plan-tooling delivery-loop alignment

## Overview
This plan aligns `crates/plan-issue-cli` and `crates/plan-tooling` with the latest plan-execution and issue-orchestration contract used by `plan-issue-delivery-loop`. It removes legacy `per-task` semantics, makes issue body sections derive from the plan preface, and adds strategy-aware split behavior (`deterministic|auto`) where group assignment rules differ. It also includes a targeted maintainability refactor to adopt `nils-common` and `nils-test-support` for repeated primitives, adds shared markdown-payload linting for GitHub body/comment content (with force override), then consolidates the `plan-issue-cli` test suite into meaningful file groupings with measurable coverage increase.

## Scope
- In scope:
  - `crates/plan-issue-cli`: issue-body rendering source sections, execution-mode semantics, strategy-aware grouping, helper refactors, test consolidation.
  - `crates/plan-tooling`: reusable split-planning API surfaces needed by `plan-issue-cli` for deterministic and auto strategy parity.
  - `crates/nils-common`: shared markdown payload lint utilities to block malformed escaped-control sequences in GitHub markdown content by default.
  - Delivery-loop compatibility behavior consumed by `plan-issue-cli` outputs (`multi-sprint-guide`, notes, and gating messages).
  - Coverage increase for `nils-plan-issue-cli` line coverage by at least `+10%` from current baseline.
- Out of scope:
  - Redesigning plan markdown format beyond current `plan-tooling` parser contract.
  - Rewriting unrelated workspace CLIs.
  - Changing GitHub workflow policy semantics outside the current plan issue/sprint gates.

## Assumptions
1. Current `nils-plan-issue-cli` baseline line coverage is `72.78%` (`cargo llvm-cov --package nils-plan-issue-cli --summary-only`).
2. `plan-tooling split-prs --strategy auto` remains deterministic and is suitable to reuse from `plan-issue-cli`.
3. The required issue body sections (`Goal`, `Acceptance Criteria`, `Scope`) should be generated from plan preface sections before `Sprint 1` (`Overview`, `Scope`, `Assumptions`, `Success criteria`) when those sections exist.
4. Sprint execution remains sequential integration gates: no cross-sprint execution parallelism.

## Success criteria
- `start-plan` issue body generation uses plan preface content for `Goal` / `Acceptance Criteria` / `Scope`, with deterministic fallback text when sections are missing.
- `per-task` is removed from `plan-issue-cli` behavior/docs/tests and replaced by `pr-isolated` uniqueness semantics.
- `plan-issue-cli` exposes strategy-aware grouping behavior aligned with `plan-tooling`:
  - `group + deterministic`: full mapping required.
  - `group + auto`: optional pin mappings plus deterministic auto-assignment for remaining tasks.
- GitHub markdown payloads (issue/pr body/comment) are rejected when they contain literal escaped-control artifacts such as `\\n`, unless force mode (`-f`/`--force`) is explicitly enabled.
- `plan-issue-cli` adopts `nils-common`/`nils-test-support` for repeated process/git/test utilities with no contract regressions.
- `crates/plan-issue-cli/tests` are reorganized into meaningful files and crate line coverage reaches at least `73.28%`.

## Issue Body Copy/Paste Seed
Use this section as the source for plan issue body `Goal / Acceptance Criteria / Scope` content.

### Goal
- Deliver the five requested changes across `plan-issue-cli` and `plan-tooling` with strict behavioral parity for orchestration gates.

### Acceptance Criteria
- Issue body sections are sourced from plan preface sections.
- Legacy `per-task` mode is removed in favor of `pr-isolated`.
- Strategy auto behavior is available and contract-aligned in `plan-issue-cli`.
- Shared-helper refactor reduces local duplicate primitives.
- Tests are consolidated and coverage increases by at least `+10%` for `nils-plan-issue-cli`.

### Scope
- In scope: `crates/plan-issue-cli`, `crates/plan-tooling`, related docs/fixtures/tests.
- Out of scope: unrelated workspace crate behavior.

## Sprint 1: Preface-derived issue body and execution-mode contract cleanup
**Goal**: Make issue-body top sections come from plan preface content and remove legacy `per-task` semantics.
**PR grouping intent**: `group` (three focused PRs).
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/plan-issue-cli-tooling-delivery-loop-alignment-plan.md`
  - `cargo test -p nils-plan-issue-cli issue_body`
  - `cargo test -p nils-plan-issue-cli sprint3_delivery`
- Verify:
  - `start-plan` issue body uses plan-preface-derived text for `Goal`, `Acceptance Criteria`, and `Scope`.
  - `Execution Mode` contract only allows `per-sprint`, `pr-isolated`, `pr-shared`, or `TBD`.
  - Duplicate branch/worktree checks trigger only for `pr-isolated`.
**Sprint scorecard**:
- `TotalComplexity`: 13
- `CriticalPathComplexity`: 13
- `MaxBatchWidth`: 1
- `OverlapHotspots`: `crates/plan-issue-cli/src/render.rs` and `crates/plan-issue-cli/src/issue_body.rs` are intentionally serialized.
**Parallelizable tasks**:
- none (intentional serial sequencing due overlapping contract files).

### Task 1.1: Derive plan issue body sections from pre-sprint plan content
- **Location**:
  - `crates/plan-issue-cli/src/render.rs`
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/tests/sprint3_delivery.rs`
- **Description**: Replace hard-coded issue body `Goal`/`Acceptance Criteria`/`Scope` strings with parser-based extraction from plan sections before `## Sprint 1`, with deterministic fallback text when sections are missing.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - `start-plan` writes issue body sections using preface text from the plan file.
  - Extraction is stable for both live and local binaries.
  - Missing preface sections fall back to deterministic default copy.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli render_issue_body_start_plan_writes_issue_body_artifact -- --exact`
  - `cargo test -p nils-plan-issue-cli local_flow_plan_issue_local_dry_run_end_to_end_generates_artifacts -- --exact`

### Task 1.2: Remove `per-task` from execution-mode validation and rendering
- **Location**:
  - `crates/plan-issue-cli/src/issue_body.rs`
  - `crates/plan-issue-cli/src/render.rs`
  - `crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md`
- **Description**: Update validation rules, rendered consistency notes, and state-machine docs so `per-task` is no longer accepted and uniqueness enforcement references only `pr-isolated`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `Execution Mode` parser rejects `per-task`.
  - Duplicate branch/worktree diagnostics mention `pr-isolated`.
  - Rendered issue-body consistency rules no longer mention `per-task`.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli validate_rows_flags_non_subagent_owner_for_done_rows -- --exact`
  - `rg -n 'per-task' crates/plan-issue-cli/src/issue_body.rs crates/plan-issue-cli/src/render.rs crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md`

### Task 1.3: Refresh fixtures and regression tests for updated execution-mode contract
- **Location**:
  - `crates/plan-issue-cli/tests/sprint4_delivery.rs`
  - `crates/plan-issue-cli/tests/sprint5_delivery.rs`
  - `crates/plan-issue-cli/tests/fixtures/shell_parity/comment_template_start.md`
  - `crates/plan-issue-cli/tests/fixtures/shell_parity/help.txt`
- **Description**: Update existing live/local regression fixtures and tests to assert `pr-isolated` behavior and new issue-body preface rendering paths.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Existing sprint delivery tests pass with updated mode semantics.
  - Shell parity fixtures remain deterministic after updates.
  - No stale `per-task` references remain in plan-issue-cli test fixtures.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test sprint4_delivery`
  - `cargo test -p nils-plan-issue-cli --test sprint5_delivery`
  - `rg -n 'per-task' crates/plan-issue-cli/tests crates/plan-issue-cli/tests/fixtures/shell_parity`

## Sprint 2: Strategy-aware split planning alignment (`deterministic|auto`)
**Goal**: Align `plan-issue-cli` grouping behavior with `plan-tooling split-prs` strategy contract and remove stale group-mode guidance.
**PR grouping intent**: `group` (isolated API/execution/docs PRs).
**Execution Profile**: `parallel-x2` (parallel width 2).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-plan-tooling --test split_prs`
  - `cargo test -p nils-plan-issue-cli --test output_contract`
  - `cargo test -p nils-plan-issue-cli --test sprint5_delivery`
- Verify:
  - `plan-issue-cli` accepts strategy input and applies strategy-specific group rules.
  - `group + auto` supports optional pin mappings and deterministic auto assignment.
  - Guide/contract messaging no longer claims full mapping is always required.
**Sprint scorecard**:
- `TotalComplexity`: 17
- `CriticalPathComplexity`: 14
- `MaxBatchWidth`: 2
- `OverlapHotspots`: `crates/plan-issue-cli/src/task_spec.rs` and `crates/plan-tooling/src/split_prs.rs` must avoid conflicting behavior during API extraction.
**Parallelizable tasks**:
- `Task 2.1` and `Task 2.2` can run in parallel before `Task 2.3`.

### Task 2.1: Add strategy option to plan-issue-cli command contract and validators
- **Location**:
  - `crates/plan-issue-cli/src/commands/mod.rs`
  - `crates/plan-issue-cli/src/commands/build.rs`
  - `crates/plan-issue-cli/src/commands/plan.rs`
  - `crates/plan-issue-cli/src/commands/sprint.rs`
  - `crates/plan-issue-cli/tests/cli_contract.rs`
- **Description**: Introduce `--strategy deterministic|auto` on split-dependent commands and enforce strategy-aware validation (`group + deterministic` strict mapping; `group + auto` optional mappings).
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - CLI parses and validates strategy argument consistently for build/start/ready/accept flows.
  - Existing default behavior remains `deterministic`.
  - Validation errors are deterministic and schema-stable.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test cli_contract`
  - `cargo test -p nils-plan-issue-cli --test output_contract output_json_contract_error_envelope_contains_code_and_message -- --exact`

### Task 2.2: Expose reusable split-planning core in plan-tooling for crate-level consumers
- **Location**:
  - `crates/plan-tooling/src/lib.rs`
  - `crates/plan-tooling/src/split_prs.rs`
  - `crates/plan-tooling/tests/split_prs.rs`
- **Description**: Extract and expose a library-level split planner that returns deterministic/grouped records for `deterministic` and `auto`, while preserving current `split-prs` CLI output behavior.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - `plan-tooling` provides a reusable API for split planning with strategy input.
  - CLI behavior and fixtures remain unchanged.
  - Auto strategy determinism remains covered by tests.
- **Validation**:
  - `cargo test -p nils-plan-tooling --test split_prs`
  - `cargo test -p nils-plan-tooling split_prs_auto_group_without_mapping_succeeds`

### Task 2.3: Refactor plan-issue-cli task-spec generation to consume plan-tooling split core
- **Location**:
  - `crates/plan-issue-cli/src/task_spec.rs`
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/tests/sprint3_delivery.rs`
- **Description**: Replace local group-assignment logic with plan-tooling split core integration, including strategy passthrough and deterministic notes projection in task-spec output.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - `plan-issue-cli` task-spec rows are produced from shared split logic.
  - `group + auto` works without full mapping when strategy is auto.
  - Deterministic/group output remains stable for existing fixtures.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test sprint3_delivery strategy_auto_partial_mapping_allows_unmapped_rows -- --exact`
  - `cargo test -p nils-plan-issue-cli --test sprint3_delivery task_spec_generation_build_task_spec_writes_grouped_rows -- --exact`
  - `cargo test -p nils-plan-issue-cli --test output_contract`

### Task 2.4: Update guide messaging, fixtures, and docs for strategy-specific group rules
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/tests/fixtures/shell_parity/multi_sprint_guide_dry_run.txt`
  - `crates/plan-tooling/README.md`
  - `crates/plan-tooling/docs/runbooks/split-prs-build-task-spec-cutover.md`
- **Description**: Remove stale message text that always requires full group mappings and replace it with strategy-specific guidance consistent with split planner behavior.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 4
- **Acceptance criteria**:
  - `multi-sprint-guide` notes describe deterministic vs auto group behavior correctly.
  - Parity fixtures and docs agree on strategy-specific semantics.
  - No stale message claims remain in plan-issue-cli and plan-tooling docs.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test sprint5_delivery parity_shell_multi_sprint_guide_matches_shell_fixture_after_normalization -- --exact`
  - `rg -n 'requires mapping for every task|requires at least one --pr-group' crates/plan-issue-cli/src crates/plan-issue-cli/tests/fixtures/shell_parity crates/plan-tooling/README.md crates/plan-tooling/docs/runbooks/split-prs-build-task-spec-cutover.md`

## Sprint 3: Shared helper refactor using nils-common / nils-test-support
**Goal**: Remove repeated process/git/test helper code in plan-issue-cli by adopting shared workspace helpers, and add shared markdown payload lint + force override for GitHub content writes.
**PR grouping intent**: `group` (runtime lane + tests lane + markdown-guard lane).
**Execution Profile**: `parallel-x3` (parallel width 3).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-plan-issue-cli`
  - `cargo test -p nils-plan-tooling --test split_prs`
  - `cargo test -p nils-common markdown_payload`
- Verify:
  - Repo-root and git-process helpers are shared via `nils-common`.
  - Test command/path/env helper duplication is reduced via `nils-test-support`.
  - GitHub markdown content writes fail fast on malformed literal escape artifacts by default, with explicit `-f` force override.
  - CLI behavior and error text contracts remain stable.
**Sprint scorecard**:
- `TotalComplexity`: 22
- `CriticalPathComplexity`: 14
- `MaxBatchWidth`: 3
- `OverlapHotspots`: batch-2 overlap includes `crates/plan-issue-cli/src/execute.rs`, `crates/plan-issue-cli/tests/sprint4_delivery.rs`, and `crates/plan-issue-cli/tests/sprint5_delivery.rs`; keep runtime/write-path edits serialized when conflicts appear.
**Parallelizable tasks**:
- `Task 3.2`, `Task 3.3`, and `Task 3.5` can run in parallel after `Task 3.1`.

### Task 3.1: Adopt nils-common repo-root and git/process helpers in plan-issue-cli core paths
- **Location**:
  - `crates/plan-issue-cli/Cargo.toml`
  - `crates/plan-issue-cli/src/task_spec.rs`
  - `crates/plan-issue-cli/src/render.rs`
  - `crates/plan-issue-cli/src/github.rs`
- **Description**: Replace local repo-root and low-level process probing patterns with `nils_common::git`/`nils_common::process` helper calls while preserving return shapes and diagnostics.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - `plan-issue-cli` depends on `nils-common`.
  - Duplicate `git rev-parse --show-toplevel` helper logic is removed from task-spec/render paths.
  - Repo slug resolution continues to pass existing behavior tests.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli github::tests::normalize_repo_slug_accepts_common_remote_forms -- --exact`
  - `rg -n 'rev-parse --show-toplevel' crates/plan-issue-cli/src/task_spec.rs crates/plan-issue-cli/src/render.rs`

### Task 3.2: Migrate worktree cleanup and git worktree calls to shared execution wrappers
- **Location**:
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/nils-common/src/git.rs`
  - `crates/nils-common/src/process.rs`
- **Description**: Route cleanup-related git command execution through shared wrappers (or shared wrapper extensions) to remove repeated subprocess plumbing and standardize failure handling.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Cleanup flow uses shared wrappers for git worktree listing/removal/prune commands.
  - Error surfaces remain deterministic for dry-run and live paths.
  - No regressions in close-plan cleanup gating behavior.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test sprint4_delivery live_plan_commands_ready_and_close_follow_gate_contracts -- --exact`
  - `cargo test -p nils-plan-issue-cli --test sprint5_delivery command_guardrails_close_plan_requires_body_file_in_dry_run_mode -- --exact`

### Task 3.3: Consolidate integration test harness around nils-test-support command utilities
- **Location**:
  - `crates/plan-issue-cli/tests/common.rs`
  - `crates/plan-issue-cli/tests/sprint4_delivery.rs`
  - `crates/plan-issue-cli/tests/sprint5_delivery.rs`
  - `crates/nils-test-support/src/cmd.rs`
- **Description**: Replace local ad-hoc env/path command harness patterns with `nils_test_support::cmd::CmdOptions` and shared helper flows, keeping test readability and determinism.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Test harness centralizes repeated env/path command setup.
  - GH stub tests still pass without shell-environment flakiness.
  - Shared helper usage is explicit and documented in test helpers.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test sprint4_delivery`
  - `cargo test -p nils-plan-issue-cli --test sprint5_delivery`

### Task 3.4: Add characterization tests for helper-migration parity
- **Location**:
  - `crates/plan-issue-cli/tests/output_contract.rs`
  - `crates/plan-issue-cli/tests/cli_contract.rs`
  - `crates/plan-issue-cli/tests/sprint4_delivery.rs`
- **Description**: Add focused parity tests that pin error messages and key output rows affected by helper migration so runtime behavior cannot drift silently.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
- **Complexity**: 3
- **Acceptance criteria**:
  - Contract tests cover helper-migration-sensitive outputs.
  - At least one live-path and one local-path characterization test is added.
  - Parity expectations are deterministic and non-flaky.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli --test output_contract`
  - `cargo test -p nils-plan-issue-cli --test cli_contract`

### Task 3.5: Add shared markdown payload guard for GitHub body/comment writes with force override
- **Location**:
  - `crates/nils-common/src/lib.rs`
  - `crates/nils-common/src/markdown.rs`
  - `crates/plan-issue-cli/src/cli.rs`
  - `crates/plan-issue-cli/src/commands/mod.rs`
  - `crates/plan-issue-cli/src/commands/plan.rs`
  - `crates/plan-issue-cli/src/commands/sprint.rs`
  - `crates/plan-issue-cli/src/execute.rs`
  - `crates/plan-issue-cli/src/github.rs`
  - `crates/plan-issue-cli/tests/sprint4_delivery.rs`
  - `crates/plan-issue-cli/tests/sprint5_delivery.rs`
- **Description**: Implement a shared markdown payload validator in `nils-common` that detects literal escaped-control artifacts (for example `\\n`, `\\r`, `\\t`) before GitHub issue/pr body/comment writes. Integrate this guard in `plan-issue-cli` GitHub write paths, return actionable errors that explain why content is rejected, and add `-f`/`--force` command support to bypass this guard when intentionally needed.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Shared markdown validation helper exists in `nils-common` and is reusable by other crates.
  - `plan-issue-cli` blocks malformed markdown payload writes by default with explicit reason text.
  - `-f`/`--force` bypasses markdown guard for intentional exceptional payloads.
  - Dry-run and local-only read paths are unaffected; only outgoing GitHub markdown writes are guarded.
- **Validation**:
  - `cargo test -p nils-common markdown_payload`
  - `cargo test -p nils-plan-issue-cli --test sprint4_delivery github_adapter_rejects_literal_escaped_newline_without_force -- --exact`
  - `cargo test -p nils-plan-issue-cli --test sprint4_delivery github_adapter_force_flag_allows_literal_escaped_newline -- --exact`

## Sprint 4: Test-suite consolidation and coverage uplift
**Goal**: Replace ambiguous sprint-numbered test file naming with domain-based suites and raise plan-issue-cli coverage by at least `+10%`.
**PR grouping intent**: `group` (layout refactor + new tests + coverage gate).
**Execution Profile**: `serial` (parallel width 1).
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-plan-issue-cli`
  - `cargo llvm-cov --package nils-plan-issue-cli --summary-only`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify:
  - Test files are renamed/merged into meaningful domain categories.
  - Added tests cover preface extraction, strategy-auto behavior, `pr-isolated` validation paths, and markdown-payload guard/force flows.
  - Line coverage for `nils-plan-issue-cli` is at least `73.28%`.
**Sprint scorecard**:
- `TotalComplexity`: 13
- `CriticalPathComplexity`: 13
- `MaxBatchWidth`: 1
- `OverlapHotspots`: `crates/plan-issue-cli/tests/common.rs` and consolidated test modules are intentionally serialized during rename + migration.
**Parallelizable tasks**:
- none (intentional serial sequencing to avoid rename conflicts).

### Task 4.1: Consolidate and rename plan-issue-cli integration test files by behavior domain
- **Location**:
  - `crates/plan-issue-cli/tests/task_spec_flow.rs`
  - `crates/plan-issue-cli/tests/live_issue_ops.rs`
  - `crates/plan-issue-cli/tests/parity_guardrails.rs`
  - `crates/plan-issue-cli/tests/common.rs`
- **Description**: Replace `sprint3_delivery.rs`, `sprint4_delivery.rs`, and `sprint5_delivery.rs` naming with domain-based suites (for example `task_spec_flow.rs`, `live_issue_ops.rs`, `parity_and_guardrails.rs`) and move shared helpers accordingly.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Test file names clearly describe behavior under test.
  - Existing test intent remains preserved after rename/merge.
  - Build/test discovery works without stale module references.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli -- --list | rg 'task_spec|live|parity|guardrail'`
  - `cargo test -p nils-plan-issue-cli`

### Task 4.2: Add missing tests for requested behavior changes
- **Location**:
  - `crates/plan-issue-cli/tests/task_spec_flow.rs`
  - `crates/plan-issue-cli/tests/live_issue_ops.rs`
  - `crates/plan-issue-cli/tests/parity_guardrails.rs`
  - `crates/plan-tooling/tests/split_prs.rs`
- **Description**: Add tests for preface-driven issue body generation, `group + auto` behavior in plan-issue flows, `group + deterministic` mapping gate errors, `pr-isolated` duplicate branch/worktree enforcement, and markdown payload lint/force bypass behavior.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 5
- **Acceptance criteria**:
  - New tests directly cover all five requested change themes.
  - Strategy-specific behavior differences are explicit in test assertions.
  - Regression tests remain deterministic across repeated runs.
- **Validation**:
  - `cargo test -p nils-plan-issue-cli`
  - `cargo test -p nils-plan-tooling --test split_prs`

### Task 4.3: Enforce and document coverage delta for plan-issue-cli (`+10%` minimum)
- **Location**:
  - `crates/plan-issue-cli/tests/task_spec_flow.rs`
  - `crates/plan-issue-cli/tests/parity_guardrails.rs`
  - `docs/plans/plan-issue-cli-tooling-delivery-loop-alignment-plan.md`
  - `$AGENT_HOME/out/plan-issue-cli-coverage/summary.txt`
- **Description**: Capture baseline vs final `cargo llvm-cov` line coverage for `nils-plan-issue-cli`, using baseline `72.78%` from this plan. Fail delivery unless final `TOTAL` line coverage is at least `73.28%`, and store a concise coverage summary artifact.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Coverage baseline and final value are recorded and reproducible.
  - Final line coverage is `>= 73.28%`.
  - Coverage command output path is documented for reviewers.
- **Validation**:
  - `mkdir -p "$AGENT_HOME/out/plan-issue-cli-coverage"`
  - `cargo llvm-cov --package nils-plan-issue-cli --summary-only | tee "$AGENT_HOME/out/plan-issue-cli-coverage/summary.txt"`
  - `rg -n 'TOTAL' "$AGENT_HOME/out/plan-issue-cli-coverage/summary.txt"`
  - `python3 -c 'import os,re,pathlib; p=pathlib.Path(os.environ.get(\"AGENT_HOME\", str(pathlib.Path.home()/\".agents\"))) / \"out\" / \"plan-issue-cli-coverage\" / \"summary.txt\"; t=p.read_text(encoding=\"utf-8\"); m=re.search(r\"TOTAL\\s+\\d+\\s+\\d+\\s+[0-9.]+%\\s+\\d+\\s+\\d+\\s+[0-9.]+%\\s+\\d+\\s+\\d+\\s+([0-9.]+)%\", t); assert m, \"unable to parse TOTAL line coverage\"; v=float(m.group(1)); assert v>=73.28, f\"line coverage {v:.2f}% is below required 73.28%\"; print(f\"line coverage gate passed: {v:.2f}%\")'`

## Testing Strategy
- Unit:
  - `issue_body` mode/placeholder/uniqueness validation.
  - strategy-aware grouping validation and token normalization.
- Integration:
  - `plan-issue-cli` end-to-end command tests (live/local, guide flow, start/ready/accept/close gate paths).
  - `plan-tooling split_prs` deterministic/auto behavior matrix tests.
- E2E/manual:
  - dry-run loop rehearsal via `multi-sprint-guide`.
  - required workspace checks via `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`.

## Risks & gotchas
- Moving `plan-issue-cli` task splitting onto shared plan-tooling APIs can unintentionally alter notes ordering or PR-group anchors; parity tests must pin both content and ordering.
- Replacing `per-task` with `pr-isolated` touches validation, docs, and fixtures simultaneously; partial rollout would cause brittle failures.
- Test-file consolidation can hide missing scenarios unless coverage and test inventory checks are explicit.
- Shared-helper refactors in command execution paths can shift error wording; characterization tests must lock externally consumed diagnostics.

## Rollback plan
1. Revert Sprint 2 strategy-integration PRs first if grouping behavior regresses; keep deterministic-only behavior by default while retaining parser compatibility.
2. Revert Sprint 3 helper-migration commits independently if process/git wrapper changes alter runtime diagnostics.
3. If Sprint 4 consolidation causes flaky coverage or missing tests, restore previous test module split and re-apply only added high-value tests.
4. Re-run `cargo test -p nils-plan-issue-cli`, `cargo test -p nils-plan-tooling --test split_prs`, and required checks after each rollback step before re-attempting forward changes.
