# plan-issue State Machine and Gate Invariants v1

## Purpose
Define the v1 state machine and gate invariants for the Rust replacement of `plan-issue-delivery-loop.sh`.

This document is normative for lifecycle transitions that must remain compatible with the current shell orchestration flow.

## Scope
- In scope:
  - Plan-level lifecycle states and transition commands.
  - Sprint-level transition gates.
  - Task-row invariants required by issue body validation and close gates.
- Out of scope:
  - Human/JSON output envelope formatting.
  - Internal Rust module structure.

## Canonical State Objects
- Plan issue: one GitHub issue for the full plan lifecycle.
- Task rows: markdown table rows under `## Task Decomposition`.
- Sprint set: tasks associated with sprint `N` by either:
  - `Notes` token `sprint=S<N>`, or
  - task id pattern `S<N>T<k>`.

## Plan Lifecycle State Machine

States:
- `PLAN_UNSTARTED`: no plan issue exists yet.
- `PLAN_OPEN`: plan issue exists and is open.
- `PLAN_REVIEW_READY`: final plan review has been requested.
- `PLAN_CLOSED`: issue is closed and plan lifecycle is complete.

Transitions:
- `start-plan`: `PLAN_UNSTARTED -> PLAN_OPEN`
  - Creates one issue with full task decomposition initialized to `Status=planned`, `PR=TBD`.
- `ready-plan`: `PLAN_OPEN -> PLAN_REVIEW_READY`
  - Records final review intent (label/comment behavior may vary by flags).
- `close-plan`: `PLAN_OPEN|PLAN_REVIEW_READY -> PLAN_CLOSED`
  - Requires final approval comment URL.
  - Delegates close gate checks to `close-after-review`.
  - On success, enforces worktree cleanup.

`close-plan --dry-run` is a non-mutating transition preview and does not change state.

## Sprint Lifecycle State Machine

States per sprint `N`:
- `SPRINT_NOT_STARTED`
- `SPRINT_IN_PROGRESS`
- `SPRINT_REVIEW_READY`
- `SPRINT_ACCEPTED`

Transitions:
- `start-sprint N`: `SPRINT_NOT_STARTED -> SPRINT_IN_PROGRESS`
  - Renders sprint task-spec and subagent prompts.
  - Syncs Task Decomposition execution metadata from task-spec.
  - For `N > 1`, requires previous sprint merge gate pass (see gate invariants).
- `ready-sprint N`: `SPRINT_IN_PROGRESS -> SPRINT_REVIEW_READY`
  - Posts or prints sprint-ready review artifact.
- `accept-sprint N`: `SPRINT_REVIEW_READY -> SPRINT_ACCEPTED`
  - Requires approval comment URL and merge gate pass.
  - Syncs all sprint `N` rows to `Status=done`.
  - Plan issue remains open after sprint acceptance.

## Task Row Status Machine

Allowed statuses:
- `planned`
- `in-progress`
- `blocked`
- `done`

Expected transitions:
- `planned -> in-progress` when implementation PR work starts.
- `in-progress -> done` only after acceptance/merge gates for that sprint pass.
- `planned|in-progress -> blocked` when execution is paused by external constraint.
- `blocked -> in-progress` when blocker is cleared.

Row-level status rules:
- `Status in {in-progress, done}` requires non-placeholder values for:
  - `Owner`
  - `Branch`
  - `Worktree`
  - `Execution Mode`
  - `PR`
- `Status in {planned, blocked}` may keep placeholders.

## Gate Invariants

### Issue Body Structural Invariants
- `## Task Decomposition` table must exist with at least one task row.
- Required columns:
  - `Task`, `Summary`, `Owner`, `Branch`, `Worktree`, `Execution Mode`, `PR`, `Status`, `Notes`
- `Status` must be one of `{planned, in-progress, blocked, done}`.
- `Execution Mode` must be one of `{per-sprint, pr-isolated, pr-shared}` or `TBD`.
- Execution Mode derivation rule:
  - `group + auto` that resolves to one shared PR lane for a sprint is represented as `per-sprint` (single-lane execution).
  - `group + auto|deterministic` with multiple resolved PR groups keeps `pr-shared` / `pr-isolated` per group size.
- Owner policy for non-planned/non-blocked rows:
  - must include `subagent`
  - must not reference main-agent identity.
- `pr-isolated` rows must have unique `Branch` and unique `Worktree`.

### PR Normalization and Presence Invariants
- PR references are normalized to canonical `#<number>` when possible.
- Placeholder PR values (`TBD`, `-`, empty, etc.) are invalid for:
  - sprint merge/accept gates
  - close gate
  - rows with `Status in {in-progress, done}`.

### Sprint Progression Gate (`start-sprint N`, `N > 1`)
- Previous sprint `N-1` must pass merge gate:
  - every previous-sprint row has `Status=done`
  - every previous-sprint row has concrete PR reference
  - every referenced PR is merged.

### Sprint Acceptance Gate (`accept-sprint N`)
- Requires `--approved-comment-url` in valid GitHub issue/pull comment URL format.
- For sprint `N` rows:
  - every row has concrete PR reference
  - every referenced PR is merged.
- On pass, sprint `N` row statuses are synchronized to `done`.

### Plan Close Gate (`close-plan`)
- Requires final `--approved-comment-url` in valid GitHub issue/pull comment URL format.
- Delegated `close-after-review` invariants:
  - all task rows are `Status=done` unless `--allow-not-done` is explicitly used
  - every task row has non-placeholder PR
  - every referenced PR is merged
  - subagent-owner policy passes.
- After close gate success, strict task worktree cleanup is enforced.

### Worktree Cleanup Invariants
- Cleanup targets are resolved from Task Decomposition `Branch` and `Worktree`.
- Main repository worktree must never be removed.
- Cleanup succeeds only when:
  - targeted linked worktrees are removed
  - `git worktree prune` succeeds
  - no targeted residual linked worktree/path remains.

## Dry-Run Contract
- `--dry-run` commands print intended write actions and gate traces.
- Dry-run mode must not mutate GitHub issue/PR state.
- `close-plan --dry-run` requires `--body-file` to evaluate gates locally.

## Failure Contract
- Gate violations fail the command (`exit 1`) with explicit task-row diagnostics.
- Usage/argument errors return `exit 2`.
- Successful transitions return `exit 0`.
