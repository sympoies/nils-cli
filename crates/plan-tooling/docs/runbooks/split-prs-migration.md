# split-prs Migration

## Objective
Migrate plan task splitting to `plan-tooling split-prs` grouping primitives with deterministic and
auto strategy compatibility, while moving runtime execution metadata materialization to
`plan-issue-cli`.

## Before / After
Before:

```bash
build-task-spec --plan <plan.md> --sprint <n> --pr-grouping <mode> [--pr-group <task=group>]... --task-spec-out <out.tsv>
```

After (grouping primitives):

```bash
plan-tooling split-prs \
  --file <plan.md> \
  --scope sprint \
  --sprint <n> \
  --pr-grouping <mode> \
  [--pr-group <task=group>]... \
  --strategy deterministic \
  --format json
```

Auto group example (no full manual mapping):

```bash
plan-tooling split-prs \
  --file <plan.md> \
  --scope sprint \
  --sprint <n> \
  --pr-grouping group \
  --strategy auto \
  --format json
```

## Required Compatibility
- Keep grouping output deterministic and replayable across repeated runs.
- Keep reduced TSV header exactly (when `--format tsv` is used):

```text
# task_id\tsummary\tpr_group
```

- Runtime execution metadata (`branch`, `worktree`, `owner`, `notes`) is no longer part of
  `split-prs` output and must be materialized by `plan-issue-cli` runtime lane logic.
- Preserve deterministic ordering for records and explain anchors so repeated runs are byte-stable.

## Grouping Guidance
- Use `per-sprint` when one shared PR should represent all sprint tasks.
- Use `group` for mixed isolated/shared PR shapes.
- In `pr-grouping=group` + `strategy=deterministic`, every selected task must have an explicit mapping.
- In `pr-grouping=group` + `strategy=auto`, explicit mappings are optional pins and remaining tasks are auto-assigned.

## Auto Strategy Status
- `--strategy auto` is implemented in v1 runtime.
- Grouping uses deterministic heuristics from `Complexity`, dependency topology, and `Location`.
- Explicit `--pr-group` mappings in auto/group mode are optional pins and still validated.
- Output ordering and reduced record schema remain deterministic.

## Release Gate Checklist
1. Verify deterministic command output against fixture expectations.
2. Verify auto/group output is byte-stable across repeated runs.
3. Verify downstream consumers use `plan-issue-cli` runtime materialization (instead of parsing
   removed split output runtime metadata columns/notes).
4. Verify fallback/rollback command sequence is documented before rollout.

## Deterministic Rollback Command Path
If auto rollout is disabled or deferred, keep downstream command paths pinned to deterministic mode:

```bash
plan-tooling split-prs \
  --file <plan.md> \
  --scope sprint \
  --sprint <n> \
  --pr-grouping <per-sprint|group> \
  [--pr-group <task=group>]... \
  --strategy deterministic \
  --format json
```

## Rollback
If downstream integration fails:
1. Keep `split-prs` implementation intact.
2. Revert downstream invocation wiring only.
3. Re-run fixture parity checks and reopen cutover PR once fixed.

## Emergency Switchback (Restore Auto to Not-Implemented)
Use this only if production incidents require disabling auto runtime behavior in the CLI itself.

Impacted files:
- `crates/plan-tooling/src/split_prs.rs`
- `crates/plan-tooling/tests/split_prs.rs`
- `crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
- `crates/plan-tooling/README.md`

Switchback commands:

```bash
# Revert the auto-runtime introduction commit from issue #220 Sprint 2.
git revert --no-edit e1cdc5a

# Validate fallback behavior and deterministic guardrails.
cargo test -p nils-plan-tooling --test split_prs split_prs_auto_not_implemented -- --exact
cargo test -p nils-plan-tooling --test split_prs split_prs_error_group_requires_mapping -- --exact
```
