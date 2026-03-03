# plan-issue CLI Contract v2

## Purpose

v2 defines the runtime metadata ownership model after split-prs output decoupling.

Current contract boundary:

- `plan-tooling split-prs` provides grouping primitives only (`task_id`, `summary`, `pr_group`).
- `plan-issue-cli` is the authority that materializes executable runtime metadata
  (`Owner`, `Branch`, `Worktree`, `Notes`) for task-spec artifacts and `Task Decomposition` rows.

## Runtime Metadata Materialization (v2)

`plan-issue-cli` runtime metadata is derived from:

- parsed plan tasks (`Task N.M`, dependencies, validation commands)
- split-prs grouping output (`task_id`, `summary`, `pr_group`)
- command grouping/strategy (`--strategy`, deterministic `--pr-grouping`, auto `--default-pr-grouping`)
- prefix options (`--owner-prefix`, `--branch-prefix`, `--worktree-prefix`)

Rules:

- `Owner` / `Branch` / `Worktree` are lane-canonical for shared lanes (`per-sprint`, `pr-shared`).
- `Notes` are task-specific and include shared-lane tokens when applicable.
- Anchor selection for runtime lane materialization is deterministic from lane membership
  (stable task ordering), not passthrough split-prs task placeholders.
- `strategy=deterministic` requires command `--pr-grouping`; if sprint metadata declares `PR grouping intent`, it must match or task-spec
  generation fails before issue/comment artifact writes.
- `strategy=auto` resolves each sprint from plan metadata `PR grouping intent` first and uses `--default-pr-grouping` only when metadata is
  absent.

## Notes Token Contract (v2)

Materialized `Notes` tokens include:

- `sprint=S<n>`
- `plan-task:Task N.M` (or deterministic fallback task id)
- optional `deps=...`
- optional `validate=...`
- `pr-grouping=<mode>`
- `pr-group=<group>`
- optional `shared-pr-anchor=<task_id>` for shared lanes

## Markdown Canonicalization Dependency

`plan-issue-cli` must use shared helpers from `nils-common::markdown` for:

- markdown payload validation (`validate_markdown_payload`)
- markdown-table-safe cell canonicalization (`canonicalize_table_cell`)

This prevents drift caused only by markdown table rendering/parsing normalization (`|`, `\n`, `\r`).

GitHub integration boundary:

- Live issue/pull writes remain crate-local `gh` adapter behavior (`plan-issue-cli` ownership).
- `nils-common` does not provide a shared `github` module for these operations.

## Task Decomposition Runtime-Truth Contract (v2)

- `## Task Decomposition` remains the single runtime-truth execution table.
- Task-spec TSV and subagent prompt files are derived artifacts.
- Drift checks compare issue rows against plan-issue materialized runtime metadata (not split-prs
  runtime placeholders).
- `group + auto|deterministic` single-lane sprints are normalized to `Execution Mode=per-sprint`.

## Task-spec TSV Header (unchanged)

```text
# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group
```

The header remains stable in v2; only the metadata generation authority changed.
