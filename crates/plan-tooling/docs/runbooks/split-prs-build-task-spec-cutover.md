# split-prs Build Task-Spec Cutover

## Goal
Replace downstream `build-task-spec` split generation with `plan-tooling split-prs` while preserving TSV compatibility.

## Command Mapping

Prior command shape:

```bash
build-task-spec --plan <plan.md> --sprint <n> --pr-grouping <mode> [--pr-group <task=group>]... --task-spec-out <out.tsv>
```

Equivalent with `plan-tooling`:

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

## Compatibility Contract
The generated TSV must keep this exact header:

```text
# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group
```

And preserve these notes keys:
- `sprint=S<n>`
- `plan-task:Task N.M`
- `pr-grouping=<mode>`
- `pr-group=<group>`
- optional `deps=...`
- optional `validate=...`
- optional `shared-pr-anchor=...` when a group has more than one task

## Group Mode Rules
- `--pr-grouping group` requires explicit `--pr-group` for every selected task.
- mapping key accepts either `SxTy` or plan task id (`Task N.M`).
- shared group output should map to one PR (`pr-shared` downstream execution mode).

## Parity Checks

```bash
mkdir -p "$AGENT_HOME/out/plan-issue-delivery-loop"

plan-tooling split-prs \
  --file crates/plan-tooling/tests/fixtures/split_prs/duck-plan.md \
  --scope sprint \
  --sprint 1 \
  --pr-grouping per-sprint \
  --strategy deterministic \
  --format tsv > "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s1.tsv"

plan-tooling split-prs \
  --file crates/plan-tooling/tests/fixtures/split_prs/duck-plan.md \
  --scope sprint \
  --sprint 2 \
  --pr-grouping group \
  --pr-group S2T1=s2-isolated \
  --pr-group S2T2=s2-shared \
  --pr-group S2T3=s2-shared \
  --strategy deterministic \
  --format tsv > "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s2.tsv"

rg -n '^# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group$' \
  "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s1.tsv" \
  "$AGENT_HOME/out/plan-issue-delivery-loop/duck-s2.tsv"
```

## Auto Strategy
`--strategy auto` is intentionally non-functional in v1 and returns `not implemented` with planned factors:
- `Complexity`
- `Location`
- `Dependencies`
