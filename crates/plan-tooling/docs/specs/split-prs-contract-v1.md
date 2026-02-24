# split-prs Contract v1

## Purpose
`plan-tooling split-prs` deterministically converts plan tasks into executable PR slices.

Deterministic v1 supports:
- grouping mode: `per-sprint` or `group`
- output format: `tsv` or `json`

## CLI

```text
plan-tooling split-prs \
  --file <plan.md> \
  --scope <plan|sprint> \
  [--sprint <n>] \
  --pr-grouping <per-sprint|group> \
  [--pr-group <task-or-plan-id>=<group>]... \
  [--strategy <deterministic|auto>] \
  [--owner-prefix <text>] \
  [--branch-prefix <text>] \
  [--worktree-prefix <text>] \
  [--format <tsv|json>]
```

Defaults:
- `--scope sprint`
- `--strategy deterministic`
- `--owner-prefix subagent`
- `--branch-prefix issue`
- `--worktree-prefix issue__`
- `--format json`

## Scope Rules
- `scope=sprint` requires `--sprint <n>`.
- `scope=plan` includes all sprints and ignores `--sprint`.

## Grouping Rules
- `pr-grouping=per-sprint`:
  - each sprint maps to one deterministic `pr_group` (`s<n>`)
- `pr-grouping=group`:
  - every task in selected scope must have explicit `--pr-group` mapping
  - mapping key may be generated task id (`SxTy`) or plan task id (`Task N.M`)

## Deterministic Normalization
- Generated task id: `S<sprint>T<index-within-sprint>` (1-based)
- Summary: normalized whitespace from task heading.
- Branch slug:
  - lowercase
  - non `[a-z0-9]` replaced with `-`
  - trimmed `-`
  - fallback `task-<index>`
  - max length 48
- Group key:
  - lowercase
  - non `[a-z0-9]` replaced with `-`
  - trimmed `-`
  - fallback from scope context
  - max length 48

## TSV Output (format=tsv)
Header:

```text
# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group
```

Column contract:
- `task_id`: generated stable id (`SxTy`)
- `summary`: task summary
- `branch`: target branch name
- `worktree`: target worktree name
- `owner`: subagent owner id
- `notes`: semicolon-separated metadata tokens
- `pr_group`: resolved group key

`notes` contains:
- `sprint=S<n>`
- `plan-task:Task N.M` (or fallback task id)
- optional `deps=...`
- optional `validate=<first validation command>`
- `pr-grouping=<mode>`
- `pr-group=<group>`
- optional `shared-pr-anchor=<task_id>` when group has multiple tasks

## JSON Output (format=json)
Object shape:
- `file`: input plan path
- `scope`: `plan` or `sprint`
- `sprint`: integer or null
- `pr_grouping`: `per-sprint` or `group`
- `strategy`: `deterministic` or `auto`
- `records`: array of records with the same fields as TSV columns

## Strategy
- `strategy=deterministic`: enabled in v1.
- `strategy=auto`: **not implemented in v1**.
  - command returns non-zero
  - error message must mention planned factors exactly: `Complexity`, `Location`, `Dependencies`

## Error Matrix (deterministic)
- missing `--file`
- plan file not found
- invalid/unknown `--scope`
- `scope=sprint` without valid `--sprint`
- invalid `--pr-grouping`
- `pr-grouping=group` without mappings
- `--pr-group` used when not in `group` mode
- unknown mapping key in `--pr-group`
- missing mapping for any selected task in `group` mode
- empty selected scope (no tasks)

## Exit Codes
- `0`: success
- `1`: runtime/validation failure
- `2`: usage error
