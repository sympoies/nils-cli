# plan-issue-cli

## Overview
`plan-issue-cli` provides the Rust command contract for plan/issue delivery orchestration.
It is the typed replacement lane for `plan-issue-delivery-loop.sh` behavior and is built around
deterministic task-spec generation, issue-body rendering, and gate-enforced sprint transitions.

The crate ships two binaries with the same command surface:

- `plan-issue`: live GitHub-backed mode
- `plan-issue-local`: local-first rehearsal mode (offline/dry-run friendly)

Shell wrapper scripts are deprecated for this crate path. Use `plan-issue` / `plan-issue-local` directly.

## Command surface

### Build and preparation
- `build-task-spec`: build sprint-scoped task-spec TSV from a plan.
- `build-plan-task-spec`: build plan-scoped task-spec TSV (all sprints).

### Plan-level flow
- `start-plan`: open one plan issue and emit plan artifacts.
- `status-plan`: summarize Task Decomposition status from issue body/body file.
- `ready-plan`: apply review-ready markers and optional review summary comment.
- `close-plan`: enforce final close gate and close the plan issue.
- `cleanup-worktrees`: enforce cleanup of all issue-assigned task worktrees.

### Sprint-level flow
- `start-sprint`: open sprint execution loop after previous sprint gate passes.
- `ready-sprint`: post sprint-ready signal for main-agent review.
- `accept-sprint`: enforce merged-PR gate and mark sprint accepted.
- `multi-sprint-guide`: print repeated command flow for a whole plan.

### Shell completion
- `completion <bash|zsh>`: export completion script for each binary.

## Global flags
- `--repo <owner/repo>`: pass-through repo target for GitHub operations.
- `--dry-run`: print write actions without mutating GitHub state.
- `-f, --force`: bypass markdown payload guard for body/comment writes.
- `--json` or `--format json`: machine-readable contract output.
- `--format text`: human-readable output.

## Local-mode constraints
- `plan-issue-local` does not support live `--issue` paths that require GitHub reads/writes.
- Use `plan-issue <command>` for live operations.
- Use `--body-file` + `--dry-run` flows for local rehearsal where supported.
- `start-plan` in local mode emits deterministic placeholder issue number `999`.

## Task Decomposition schema
- Canonical table columns are fixed to:
  - `Task | Summary | Owner | Branch | Worktree | Execution Mode | PR | Status | Notes`
- Writer and parser share the same schema contract.
- Writer sanitizes cell values (including `|`) so parser column count remains deterministic.

## Grouping and strategy rules
- `--pr-grouping` is required for split-dependent commands:
  - `build-task-spec`, `build-plan-task-spec`, `start-plan`, `start-sprint`, `ready-sprint`, `accept-sprint`.
- `--pr-grouping per-sprint`: one shared group per sprint (default style).
- `--pr-grouping group --strategy deterministic`: requires explicit `--pr-group <task>=<group>` mappings.
- `--pr-grouping group --strategy auto`: allows optional pins and auto assignment for remaining tasks.

## Quick examples
```bash
# 1) Build plan-scoped task spec locally
plan-issue-local build-plan-task-spec \
  --plan docs/plans/example-plan.md \
  --pr-grouping per-sprint

# 2) Start plan issue in live mode
plan-issue start-plan \
  --repo owner/repo \
  --plan docs/plans/example-plan.md \
  --pr-grouping per-sprint

# 3) Local rehearsal start-plan (deterministic placeholder issue_number=999)
plan-issue-local --format json --dry-run start-plan \
  --plan docs/plans/example-plan.md \
  --pr-grouping per-sprint

# 4) Export completion
plan-issue completion zsh > completions/zsh/_plan-issue
plan-issue-local completion bash > completions/bash/plan-issue-local
```

## Exit codes
- `0`: success
- `1`: runtime/validation failure
- `2`: usage failure

## Specifications
- [CLI contract v1](docs/specs/plan-issue-cli-contract-v1.md)
- [State machine and gate invariants v1](docs/specs/plan-issue-state-machine-v1.md)
- [Gate matrix v1](docs/specs/plan-issue-gate-matrix-v1.md)

## Fixtures
- Shell parity fixtures live under `tests/fixtures/shell_parity/`.
- Use `tests/fixtures/shell_parity/regenerate.sh` to refresh fixture snapshots when shell behavior intentionally changes.
