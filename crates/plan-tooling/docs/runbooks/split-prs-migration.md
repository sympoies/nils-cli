# split-prs Migration

## Objective
Migrate plan task splitting from embedded downstream logic to `plan-tooling split-prs` with deterministic output compatibility.

## Before / After
Before:

```bash
build-task-spec --plan <plan.md> --sprint <n> --pr-grouping <mode> [--pr-group <task=group>]... --task-spec-out <out.tsv>
```

After:

```bash
plan-tooling split-prs \
  --file <plan.md> \
  --scope sprint \
  --sprint <n> \
  --pr-grouping <mode> \
  [--pr-group <task=group>]... \
  --strategy deterministic \
  --format tsv > <out.tsv>
```

## Required Compatibility
- Keep TSV header exactly:

```text
# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group
```

- Keep notes keys expected by downstream orchestration:
  - `sprint=S<n>`
  - `plan-task:Task N.M`
  - `pr-grouping=<mode>`
  - `pr-group=<group>`
  - optional `deps=...`
  - optional `validate=...`
  - optional `shared-pr-anchor=...`

## Grouping Guidance
- Use `per-sprint` when one shared PR should represent all sprint tasks.
- Use `group` for mixed isolated/shared PR shapes.
- In `group` mode every selected task must have an explicit mapping.

## Auto Strategy Status
- `--strategy auto` is intentionally not implemented in v1.
- Current message references planned scoring factors: `Complexity`, `Location`, `Dependencies`.

## Release Gate Checklist
1. Verify deterministic command output against fixture expectations.
2. Verify downstream consumers still parse TSV columns/notes without changes.
3. Verify fallback/rollback command sequence is documented before rollout.

## Rollback
If downstream integration fails:
1. Keep `split-prs` implementation intact.
2. Revert downstream invocation wiring only.
3. Re-run fixture parity checks and reopen cutover PR once fixed.
