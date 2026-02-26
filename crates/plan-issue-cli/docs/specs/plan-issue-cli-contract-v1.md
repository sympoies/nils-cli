# plan-issue CLI Contract v1

> Historical reference: see `plan-issue-cli-contract-v2.md` for the current runtime metadata
> ownership model where `split-prs` outputs grouping primitives only and `plan-issue-cli`
> materializes runtime execution metadata.

## Purpose
Define the v1 command contract for the Rust replacement of the current plan issue orchestration shell entrypoint (`plan-issue-delivery-loop.sh`).

This contract is the Sprint 1 source of truth for command names, required flags, gating behavior, and deterministic task-spec compatibility.

## Scope
- In scope:
  - Canonical command surface and subcommand matrix.
  - Shared option rules (`--repo`, `--dry-run`, grouping flags, summary flags).
  - Deterministic task-spec output compatibility and default artifact locations.
  - Gate-oriented behavior for sprint and plan transitions.
- Out of scope for this document:
  - Internal state machine details (tracked in `plan-issue-state-machine-v1.md`).
  - Full text/JSON output envelope details (expanded in later implementation tasks).
  - GitHub API implementation internals.

## CLI Surface (v1)

```text
plan-issue <subcommand> [options]
```

v1 subcommands:
- `build-task-spec`
- `build-plan-task-spec`
- `start-plan`
- `status-plan`
- `ready-plan`
- `close-plan`
- `cleanup-worktrees`
- `start-sprint`
- `ready-sprint`
- `accept-sprint`
- `multi-sprint-guide`

## Command Matrix

| Subcommand | Scope | GitHub dependency | Required core inputs | Primary outputs |
| --- | --- | --- | --- | --- |
| `build-task-spec` | Sprint-scoped task split preview | no | `--plan`, `--sprint`, `--pr-grouping` | sprint TSV task-spec |
| `build-plan-task-spec` | Plan-scoped task split preview | no | `--plan`, `--pr-grouping` | plan TSV task-spec |
| `start-plan` | Initialize one plan issue | yes (unless dry-run) | `--plan`, `--pr-grouping` | issue creation + rendered issue body artifact |
| `status-plan` | Plan issue status snapshot | yes (unless `--body-file`) | `--issue` or `--body-file` | status summary and optional comment |
| `ready-plan` | Request final plan review | yes (unless `--body-file` only mode) | `--issue` or `--body-file` | review-ready signal (+ label/comment controls) |
| `close-plan` | Final gate + issue close | yes (except dry-run with `--body-file`) | `--approved-comment-url` and issue context | issue close + required worktree cleanup |
| `cleanup-worktrees` | Remove all issue-assigned worktrees | yes (to read issue body) | `--issue` | deleted worktree set (or dry-run listing) |
| `start-sprint` | Begin sprint execution on existing plan issue | yes (unless dry-run) | `--plan`, `--issue`, `--sprint`, `--pr-grouping` | sprint TSV, rendered subagent prompts, issue-row runtime-truth validation, kickoff comment |
| `ready-sprint` | Request sprint acceptance review | yes | `--plan`, `--issue`, `--sprint` | sprint review-ready comment |
| `accept-sprint` | Enforce merged-PR gate and mark sprint done | yes | `--plan`, `--issue`, `--sprint`, `--approved-comment-url` | task status sync to `done` + acceptance comment |
| `multi-sprint-guide` | Print repeatable command flow | no (with `--dry-run`) | `--plan` | execution guide text |

## Shared Flag and Validation Rules

- `--repo <owner/repo>`: pass-through repository target for GitHub operations.
- `--dry-run`: prints write actions without mutating GitHub state.
- `--pr-grouping <per-sprint|group>`:
  - required for `build-task-spec`, `build-plan-task-spec`, `start-plan`, `start-sprint`.
  - `per-spring` must be accepted as compatibility alias for `per-sprint`.
  - with `--pr-grouping group --strategy auto|deterministic`, when a sprint resolves to exactly one shared PR group, runtime-truth/render paths normalize `Execution Mode` to `per-sprint` (single-lane semantics).
- `--pr-group <task=group>`:
  - repeatable.
  - valid only when `--pr-grouping group`.
  - task key may be generated task id (`SxTy`) or plan task id (`Task N.M`).
- `--summary` and `--summary-file` are mutually exclusive where provided (`ready-plan`, `ready-sprint`).
- `close-plan --dry-run` requires `--body-file` when no live issue read is available.

## Task Decomposition Runtime-Truth Contract (v1)

- `## Task Decomposition` in the plan issue body is the single runtime-truth execution table for plan/sprint orchestration.
- No second issue-body dispatch table is introduced in v1; task dispatch artifacts are derived from `Task Decomposition`.
- Column roles are split as follows:
  - runtime-truth execution columns: `Owner`, `Branch`, `Worktree`, `Execution Mode`, and lane metadata tokens in `Notes`.
  - runtime-progress columns: `PR`, `Status` (may remain placeholders until execution/review advances).
  - descriptive row identity columns: `Task`, `Summary`.
- `Owner` stores a stable dispatch alias (for example `subagent-s1-t1`, or a shared-lane `dispatch` alias), not a platform-internal ephemeral spawned-agent identifier.
- `task-spec` TSV rows and subagent prompt artifacts must be derived from the same `Task Decomposition` runtime-truth rows and must not intentionally diverge from the issue table.
- Lane canonicalization rules:
  - rows that share one execution lane (`per-sprint` or `pr-shared`) must keep canonical lane metadata (`Owner`, `Branch`, `Worktree`, lane-note tokens) synchronized across the lane.
  - `--pr-grouping group --strategy auto|deterministic` single-lane sprints normalize to `Execution Mode=per-sprint` and use canonical per-sprint lane metadata rather than per-task pseudo-lanes.

## Deterministic Artifact Contracts

### Task-spec TSV
Header must remain exactly:

```text
# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group
```

Notes field must preserve orchestration metadata tokens used by issue-table sync:
- `sprint=S<n>`
- `plan-task:Task N.M` (or deterministic fallback id)
- optional `deps=...`
- optional `validate=...`
- `pr-grouping=<mode>`
- `pr-group=<group>`
- optional `shared-pr-anchor=<task_id>`

### Default output paths
When explicit output paths are omitted, v1 keeps AGENT_HOME-based deterministic defaults:
- plan task-spec:
  - `$AGENT_HOME/out/plan-issue-delivery/<plan-stem>-plan-tasks.tsv`
- sprint task-spec:
  - `$AGENT_HOME/out/plan-issue-delivery/<plan-stem>-sprint-<n>-tasks.tsv`
- plan issue body artifact:
  - `$AGENT_HOME/out/plan-issue-delivery/<plan-stem>-plan-issue-body.md`
- sprint prompt directory:
  - `$AGENT_HOME/out/plan-issue-delivery/<plan-stem>-sprint-<n>-subagent-prompts/`

## Gate Semantics (v1)

- Single-plan issue model: one plan maps to one GitHub issue for the full delivery lifecycle.
- `Task Decomposition` runtime-truth ownership:
  - sprint execution and issue-sync flows read runtime-truth lane metadata from the issue table.
  - task-spec and prompt generation are derived outputs, not an alternate source of runtime execution truth.
- Sprint ordering gate:
  - `start-sprint` for sprint `N>1` is blocked until sprint `N-1` has merged PRs and `done` task statuses.
- Acceptance gate:
  - `accept-sprint` requires approval comment URL and merged PRs for that sprint.
- Final close gate:
  - `close-plan` requires final approval comment URL and close-gate checks to pass before issue close.
  - successful `close-plan` must enforce task worktree cleanup.

## Exit Code Contract

- `0`: success.
- `1`: runtime failure, dependency failure, or gate failure.
- `2`: usage/argument error.

## Compatibility Boundary

- Until full Rust cutover is complete, behavior must preserve user-visible orchestration semantics from the shell workflow:
  - command names,
  - required flags and gate inputs,
  - task-spec TSV compatibility,
  - sprint/plan gate outcomes.
