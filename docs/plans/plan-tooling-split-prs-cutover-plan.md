# Plan: plan-tooling split-prs deterministic V1 cutover

## Overview
This plan delivers a new `plan-tooling split-prs` subcommand that deterministically converts plan tasks into executable PR slices and emits machine-consumable `json` or `tsv`. The deterministic V1 target is parity with the current `build-task-spec` split behavior used by the plan issue delivery loop, so downstream automation can consume the new command without behavioral surprises. The initial auto-splitting entrypoint is included but intentionally returns `not implemented`, while explicitly documenting the future scoring factors (`Complexity`, `Location`, `Dependencies`). The rollout is staged so the CLI contract lands first, deterministic parity is proven second, and downstream cutover happens only after fixtures and regression checks are stable.

## Scope
- In scope:
  - Add `plan-tooling split-prs` subcommand in `plan-tooling`.
  - Support deterministic V1 split modes: `per-sprint` and `group`.
  - Support output formats: `json` and `tsv`.
  - Preserve task-spec compatibility required by the plan issue delivery loop.
  - Add an auto mode entrypoint that returns an explicit `not implemented` response mentioning `Complexity`, `Location`, and `Dependencies`.
  - Replace `plan-issue-delivery-loop.sh build-task-spec` split generation to call `plan-tooling split-prs`.
- Out of scope:
  - Implementing automatic split decisions in this phase.
  - Redesigning issue lifecycle policies (`Execution Mode`, merge gates, close gates).
  - Changing plan markdown format requirements beyond what `plan-tooling validate` already enforces.

## Assumptions (if any)
1. `plan-tooling` remains the canonical parser for Plan Format v1 and is available on PATH in downstream environments.
2. Existing downstream workflows can accept unchanged task-spec TSV columns (`task_id`, `summary`, `branch`, `worktree`, `owner`, `notes`, `pr_group`).
3. `group` mode keeps explicit mapping requirements for every task in scope.
4. The cutover is acceptable as a two-repo sequence: nils-cli ships the command first, then downstream scripts adopt it.

## Success criteria
- `plan-tooling split-prs` exists and supports deterministic V1 with `--pr-grouping per-sprint|group`.
- `split-prs` outputs valid `json` and `tsv`, and TSV is directly consumable by the existing issue loop.
- Deterministic output ordering is stable across repeated runs.
- `--strategy auto` returns a clear `not implemented` message that includes planned factors: `Complexity`, `Location`, `Dependencies`.
- Downstream `build-task-spec` uses `plan-tooling split-prs` rather than embedded split logic.

## Sprint 1: Contract and parity fixtures
**Goal**: Freeze a deterministic command contract and parity fixtures before implementation.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/plan-tooling-split-prs-cutover-plan.md`
  - `rg -n 'split-prs|per-sprint|group|json|tsv|auto' crates/plan-tooling/README.md crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
- Verify:
  - Contract doc defines CLI options, output schema, and error semantics.
  - Fixture set covers both `per-sprint` and `group` behavior.
**Parallelizable tasks**:
- `Task 1.2` can start after `Task 1.1` and run in parallel with documentation polish.

### Task 1.1: Define split-prs CLI contract and output schema
- **Location**:
  - `crates/plan-tooling/README.md`
  - `crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
  - `crates/plan-tooling/src/completion.rs`
- **Description**: Define deterministic V1 CLI options and output schema, including required `--pr-grouping` behavior, `group` mapping rules, output-format guarantees, and future-facing auto mode contract.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Contract lists deterministic mode requirements for `per-sprint` and `group`.
  - Contract specifies `json` and `tsv` outputs with field-level definitions.
  - Contract states auto mode behavior as explicit `not implemented` in V1.
- **Validation**:
  - `test -f crates/plan-tooling/README.md && test -f crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
  - `rg -n 'split-prs|pr-grouping|per-sprint|group|json|tsv|strategy auto|not implemented' crates/plan-tooling/README.md crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
  - `rg -n 'Command::new\("split-prs"\)|split-prs' crates/plan-tooling/src/completion.rs`

### Task 1.2: Capture deterministic parity fixtures from current task-spec behavior
- **Location**:
  - `crates/plan-tooling/tests/fixtures/split_prs/duck-plan.md`
  - `crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.tsv`
  - `crates/plan-tooling/tests/fixtures/split_prs/group_expected.tsv`
  - `crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.json`
  - `crates/plan-tooling/tests/fixtures/split_prs/group_expected.json`
- **Description**: Add golden fixtures that represent current deterministic split behavior for both grouping modes, so V1 implementation can be validated for compatibility and stable ordering.
- **Dependencies**:
  - `Task 1.1`
- **Complexity**: 4
- **Acceptance criteria**:
  - Fixture plan includes dependencies and locations that exercise shared-group and isolated-group cases.
  - Expected outputs include both formats and match documented field ordering.
  - Fixture names and paths are stable for regression tests.
- **Validation**:
  - `test -f crates/plan-tooling/tests/fixtures/split_prs/duck-plan.md`
  - `for f in per_sprint_expected.tsv group_expected.tsv per_sprint_expected.json group_expected.json; do test -f "crates/plan-tooling/tests/fixtures/split_prs/$f"; done`
  - `rg -n '^# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group$' crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.tsv crates/plan-tooling/tests/fixtures/split_prs/group_expected.tsv`

### Task 1.3: Define deterministic normalization and error matrix
- **Location**:
  - `crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
  - `crates/plan-tooling/tests/split_prs.rs`
- **Description**: Specify and test deterministic normalization rules for task keys, group names, slug generation, and invalid-input error cases so CLI behavior is reproducible and debuggable.
- **Dependencies**:
  - `Task 1.1`
  - `Task 1.2`
- **Complexity**: 5
- **Acceptance criteria**:
  - Contract includes normalization rules for task identifiers and group keys.
  - Error matrix covers unknown task keys, missing mappings in `group` mode, and invalid option combinations.
  - Tests assert deterministic ordering and stable error text prefixes.
- **Validation**:
  - `test -f crates/plan-tooling/docs/specs/split-prs-contract-v1.md && test -f crates/plan-tooling/tests/split_prs.rs`
  - `rg -n 'normalization|error matrix|unknown task|missing mapping|deterministic ordering' crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
  - `cargo test -p nils-plan-tooling split_prs_error`

## Sprint 2: Deterministic V1 implementation in plan-tooling
**Goal**: Implement `split-prs` deterministic engine and complete command/test wiring.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-plan-tooling split_prs`
  - `cargo test -p nils-plan-tooling completion_outside_repo`
- Verify:
  - Deterministic splits and format outputs match golden fixtures.
  - Help, usage, completion, and dispatch include the new subcommand.
**Parallelizable tasks**:
- `Task 2.2` and `Task 2.3` can run in parallel after `Task 2.1`.

### Task 2.1: Implement split-prs deterministic engine and CLI plumbing
- **Location**:
  - `crates/plan-tooling/src/split_prs.rs`
  - `crates/plan-tooling/src/lib.rs`
  - `crates/plan-tooling/src/usage.rs`
  - `crates/plan-tooling/src/completion.rs`
- **Description**: Implement `split-prs` with deterministic V1 behavior (`per-sprint` and `group`), argument parsing, TSV/JSON rendering, and command dispatch wiring.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 8
- **Acceptance criteria**:
  - `plan-tooling split-prs` accepts deterministic mode arguments and validates required combinations.
  - `--format tsv` emits task-spec-compatible columns and deterministic row order.
  - `--format json` emits equivalent structured data with deterministic ordering.
  - Help and completion expose `split-prs` and its options.
- **Validation**:
  - `cargo test -p nils-plan-tooling split_prs_deterministic`
  - `cargo test -p nils-plan-tooling dispatch_help_when_no_args`
  - `plan-tooling split-prs --help | rg -n 'per-sprint|group|json|tsv|strategy'`

### Task 2.2: Add comprehensive split-prs regression tests
- **Location**:
  - `crates/plan-tooling/tests/split_prs.rs`
  - `crates/plan-tooling/tests/common.rs`
  - `crates/plan-tooling/tests/fixtures/split_prs/per_sprint_expected.tsv`
- **Description**: Add command-level tests for deterministic parity, invalid input handling, and exact output comparisons against golden fixtures.
- **Dependencies**:
  - `Task 2.1`
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover both grouping modes, both output formats, and non-happy-path validation errors.
  - Repeated command invocations produce byte-identical deterministic output.
  - Fixture diff failures are easy to diagnose from test messages.
- **Validation**:
  - `cargo test -p nils-plan-tooling split_prs`
  - `cargo test -p nils-plan-tooling to_json`
  - `cargo test -p nils-plan-tooling batches`

### Task 2.3: Add auto strategy entrypoint with explicit not-implemented response
- **Location**:
  - `crates/plan-tooling/src/split_prs.rs`
  - `crates/plan-tooling/src/usage.rs`
  - `crates/plan-tooling/tests/split_prs.rs`
- **Description**: Implement `--strategy auto` command-path validation that returns a stable `not implemented` response and documents planned factors (`Complexity`, `Location`, `Dependencies`) without performing split decisions.
- **Dependencies**:
  - `Task 2.1`
- **Complexity**: 3
- **Acceptance criteria**:
  - `--strategy auto` is accepted by argument parsing.
  - Command exits with a documented non-success code and explicit `not implemented` message.
  - Message text includes `Complexity`, `Location`, and `Dependencies` exactly once each.
- **Validation**:
  - `cargo test -p nils-plan-tooling split_prs_auto_not_implemented`
  - `plan-tooling split-prs --file crates/plan-tooling/plan-template.md --sprint 1 --strategy auto --format json || true`

## Sprint 3: Downstream cutover from build-task-spec to split-prs
**Goal**: Replace embedded task split generation in plan issue loop with `plan-tooling split-prs`.
**Demo/Validation**:
- Command(s):
  - `command -v plan-tooling && plan-tooling --version`
  - `skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh build-task-spec --plan docs/plans/duck-issue-loop-test-plan.md --sprint 1 --pr-grouping per-sprint --task-spec-out "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s1.tsv"`
  - `skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh build-task-spec --plan docs/plans/duck-issue-loop-test-plan.md --sprint 2 --pr-grouping group --pr-group S2T1=s2-isolated --pr-group S2T2=s2-shared --pr-group S2T3=s2-shared --task-spec-out "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s2.tsv"`
- Verify:
  - Output TSV remains consumable by existing issue table sync and dispatch hints.
  - Group/shared behaviors remain unchanged for the duck-plan reference flow.
**Parallelizable tasks**:
- `Task 3.2` can run in parallel with `Task 3.3` after `Task 3.1`.

### Task 3.1: Rewire build-task-spec to call plan-tooling split-prs
- **Location**:
  - `$AGENT_HOME/skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh`
- **Description**: Replace embedded split-generation logic in `build-task-spec` with a direct `plan-tooling split-prs` invocation while preserving current flags, output columns, and stderr diagnostics used by the orchestration flow.
- **Dependencies**:
  - `Task 2.2`
  - `Task 2.3`
- **Complexity**: 8
- **Acceptance criteria**:
  - `build-task-spec` no longer computes splits internally.
  - Existing `--pr-grouping` and `--pr-group` CLI behavior remains consistent.
  - Produced task-spec TSV preserves expected header and notes fields.
  - Cutover path verifies `plan-tooling` is installed and callable before split generation.
- **Validation**:
  - `command -v plan-tooling`
  - `plan-tooling --version`
  - `rg -n 'plan-tooling split-prs' "$AGENT_HOME/skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh"`
  - `skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh build-task-spec --plan docs/plans/duck-issue-loop-test-plan.md --sprint 1 --pr-grouping per-sprint --task-spec-out "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s1.tsv"`
  - `rg -n '^# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group$' "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s1.tsv"`

### Task 3.2: Remove duplicate split logic and keep only orchestration-specific transforms
- **Location**:
  - `$AGENT_HOME/skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh`
  - `$AGENT_HOME/skills/automation/plan-issue-delivery-loop/tests/test_automation_plan_issue_delivery_loop.py`
- **Description**: Delete now-redundant split derivation code and keep only orchestration-specific behavior (issue body sync, execution mode projection, comments, and merge gates).
- **Dependencies**:
  - `Task 3.1`
- **Complexity**: 6
- **Acceptance criteria**:
  - Script no longer contains duplicate task split derivation blocks.
  - Existing orchestration tests are updated to assert split-prs invocation path.
  - Regression tests keep validation for `pr-shared` and `pr-isolated` execution mode projection.
- **Validation**:
  - `python -m pytest "$AGENT_HOME/skills/automation/plan-issue-delivery-loop/tests/test_automation_plan_issue_delivery_loop.py"`
  - `rg -n 'pr-shared|pr-isolated|split-prs' "$AGENT_HOME/skills/automation/plan-issue-delivery-loop/tests/test_automation_plan_issue_delivery_loop.py"`

### Task 3.3: Prove duck-plan parity for deterministic grouping outputs
- **Location**:
  - `$AGENT_HOME/out/plan-issue-delivery-loop/duck-s1.tsv`
  - `$AGENT_HOME/out/plan-issue-delivery-loop/duck-s2.tsv`
  - `$AGENT_HOME/out/plan-issue-delivery-loop/duck-s3.tsv`
- **Description**: Run duck-plan split generation for `per-sprint` and `group` profiles and verify output parity conditions that were validated in issue #168 behavior.
- **Dependencies**:
  - `Task 3.1`
- **Complexity**: 5
- **Acceptance criteria**:
  - Sprint 1 output has one shared `pr_group`.
  - Sprint 2 output has one isolated group and one shared two-task group.
  - Sprint 3 output has three isolated groups.
- **Validation**:
  - `skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh build-task-spec --plan docs/plans/duck-issue-loop-test-plan.md --sprint 1 --pr-grouping per-sprint --task-spec-out "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s1.tsv"`
  - `skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh build-task-spec --plan docs/plans/duck-issue-loop-test-plan.md --sprint 2 --pr-grouping group --pr-group S2T1=s2-isolated --pr-group S2T2=s2-shared --pr-group S2T3=s2-shared --task-spec-out "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s2.tsv"`
  - `skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh build-task-spec --plan docs/plans/duck-issue-loop-test-plan.md --sprint 3 --pr-grouping group --pr-group S3T1=s3-a --pr-group S3T2=s3-b --pr-group S3T3=s3-c --task-spec-out "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s3.tsv"`
  - `python3 - <<'PY'
import csv, os, pathlib
base=pathlib.Path(os.environ["AGENT_HOME"]) / "out/plan-issue-delivery-loop"
for name,expect in [("duck-s1.tsv",("single",None)),("duck-s2.tsv",("mix",None)),("duck-s3.tsv",("all_isolated",None))]:
    rows=[r for r in csv.reader((base/name).open(), delimiter='\t') if r and not r[0].startswith('#')]
    groups=[r[6] for r in rows]
    if name=="duck-s1.tsv":
        assert len(set(groups))==1, groups
    elif name=="duck-s2.tsv":
        assert groups.count('s2-shared')==2 and groups.count('s2-isolated')==1, groups
    else:
        assert len(set(groups))==3, groups
print('ok')
PY`

## Sprint 4: Hardening, release readiness, and rollback guardrails
**Goal**: Finalize docs/tests and prepare safe rollout and rollback.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
- Verify:
  - Required checks pass for `nils-cli` changes.
  - Rollout guide and rollback commands are documented and rehearsable.
**Parallelizable tasks**:
- `Task 4.1` and `Task 4.2` can run in parallel after `Task 3.2` and `Task 3.3` are complete.

### Task 4.1: Update user-facing documentation and migration notes
- **Location**:
  - `crates/plan-tooling/README.md`
  - `crates/plan-tooling/docs/runbooks/split-prs-migration.md`
  - `README.md`
- **Description**: Document how to use `split-prs`, how deterministic grouping maps to execution modes, and how downstream scripts should migrate from embedded split logic.
- **Dependencies**:
  - `Task 3.2`
- **Complexity**: 4
- **Acceptance criteria**:
  - `plan-tooling` docs include deterministic examples for `per-sprint` and `group`.
  - Migration runbook includes before/after command mapping for `build-task-spec`.
  - Auto mode is documented as intentionally not implemented in V1.
- **Validation**:
  - `test -f crates/plan-tooling/README.md && test -f crates/plan-tooling/docs/runbooks/split-prs-migration.md && test -f README.md`
  - `rg -n 'split-prs|per-sprint|group|not implemented|build-task-spec' crates/plan-tooling/README.md crates/plan-tooling/docs/runbooks/split-prs-migration.md README.md`

### Task 4.2: Add non-regression checks for split-prs adoption path
- **Location**:
  - `crates/plan-tooling/tests/split_prs.rs`
  - `$AGENT_HOME/skills/automation/plan-issue-delivery-loop/tests/test_automation_plan_issue_delivery_loop.py`
- **Description**: Add regression checks that fail if deterministic output schema drifts or downstream script stops invoking `split-prs`.
- **Dependencies**:
  - `Task 3.2`
  - `Task 3.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Regression tests pin TSV header and required notes keys.
  - Downstream tests assert command invocation route and retained execution-mode semantics.
  - Test failures provide actionable mismatch context.
- **Validation**:
  - `cargo test -p nils-plan-tooling split_prs`
  - `python -m pytest "$AGENT_HOME/skills/automation/plan-issue-delivery-loop/tests/test_automation_plan_issue_delivery_loop.py"`

### Task 4.3: Execute release gate checklist and rollback rehearsal
- **Location**:
  - `docs/plans/plan-tooling-split-prs-cutover-plan.md`
  - `crates/plan-tooling/docs/runbooks/split-prs-migration.md`
  - `$AGENT_HOME/out/plan-tooling-split-prs-cutover/release-checklist.md`
- **Description**: Run required checks, capture release/cutover checklist artifacts, and rehearse rollback commands to restore previous split behavior if downstream issues appear.
- **Dependencies**:
  - `Task 4.1`
  - `Task 4.2`
- **Complexity**: 5
- **Acceptance criteria**:
  - Required checks and coverage gate commands complete successfully.
  - Checklist artifact records versions, commands, and pass/fail results.
  - Rollback section includes specific command sequence for reverting downstream script usage to pre-cutover behavior.
- **Validation**:
  - `mkdir -p "$AGENT_HOME/out/plan-tooling-split-prs-cutover"`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage && cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Testing Strategy
- Unit:
  - Add focused unit tests in `crates/plan-tooling/tests/split_prs.rs` for argument validation, grouping normalization, deterministic ordering, and auto not-implemented behavior.
- Integration:
  - Compare `split-prs` outputs against golden fixtures for both `per-sprint` and `group`.
  - Validate downstream script `build-task-spec` consumes `split-prs` output without changing issue table sync behavior.
- E2E/manual:
  - Run duck-plan split flows for sprints 1-3 with expected group outcomes.
  - Execute `start-sprint` and `accept-sprint` dry-run checks to verify execution mode mapping remains stable after cutover.

## Risks & gotchas
- Contract drift risk: minor field-order or note-key changes in TSV can break downstream parsers silently.
- Two-repo coordination risk: cutover requires compatible `plan-tooling` version in downstream runtime.
- Determinism risk: unstable sorting of tasks or groups can create noisy diffs and PR mapping churn.
- Auto-mode ambiguity risk: users may assume `auto` makes decisions; message must clearly state non-implementation status.
- Validation surface risk: downstream tests may pass while edge-case mapping errors remain unless fixtures include invalid-key and missing-mapping cases.

## Rollback plan
- Keep deterministic split logic migration isolated so rollback can revert only split-prs wiring and leave unrelated orchestration behavior untouched.
- If downstream cutover causes issues, revert the `build-task-spec` invocation change to prior embedded logic in one patch and pin `plan-tooling` usage docs to deterministic-only guidance.
- Preserve golden fixtures and regression tests so rollback validation can prove restored behavior quickly.
- If `split-prs` CLI introduces regressions, disable downstream adoption first, then patch `plan-tooling` in a follow-up release and re-run parity fixtures before re-cutover.
