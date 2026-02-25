# split-prs Contract v2

## Purpose
`plan-tooling split-prs` emits deterministic grouping primitives for downstream orchestrators.

v2 intentionally removes task-level runtime execution metadata from `split-prs` output.
`plan-issue-cli` materializes runtime lane metadata from:

- parsed plan task content
- split-prs grouping results (`task_id`, `summary`, `pr_group`)
- command prefixes / grouping strategy

`split-prs-contract-v1.md` remains the historical reference for pre-v2 output shape.

## CLI

```text
plan-tooling split-prs \
  --file <plan.md> \
  --scope <plan|sprint> \
  [--sprint <n>] \
  --pr-grouping <per-sprint|group> \
  [--pr-group <task-or-plan-id>=<group>]... \
  [--strategy <deterministic|auto>] \
  [--explain] \
  [--format <tsv|json>]
```

Value options accept both `--key value` and `--key=value`.

Compatibility note:
- Runtime-prefix compatibility options are still accepted by the CLI parser for older
  automation, but v2 `split-prs` output is grouping-only. Runtime execution metadata is generated in
  `plan-issue-cli`.

## Determinism and Grouping Rules
- Output ordering remains deterministic: sprint number, then task appearance order.
- `pr_group` naming remains normalized and deterministic.
- `strategy=deterministic` + `pr-grouping=group` requires explicit `--pr-group` mapping for every task.
- `strategy=auto` + `pr-grouping=group` allows optional pin mappings and auto-assigns the rest.
- `strategy=auto` + `pr-grouping=per-sprint` still emits one shared `pr_group` per sprint.

## TSV Output (format=tsv)

Header:

```text
# task_id\tsummary\tpr_group
```

Columns:
- `task_id`: generated stable id (`SxTy`)
- `summary`: normalized task summary
- `pr_group`: resolved deterministic group key

## JSON Output (format=json)

Top-level object:
- `file`
- `scope`
- `sprint`
- `pr_grouping`
- `strategy`
- `records`
- optional `explain` (only with `--explain`)

`records[]` fields:
- `task_id`
- `summary`
- `pr_group`

`explain` continues to expose per-sprint grouping breakdown (`groups[].task_ids`, deterministic
anchor task id, and optional sprint metadata hints).

## Migration Notes (v1 -> v2)

Removed `split-prs` runtime metadata fields are now produced inside `plan-issue-cli` from plan task
metadata, grouping results, and prefix inputs.

## Exit Codes
- `0`: success
- `1`: runtime / validation failure
- `2`: usage error
