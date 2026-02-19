---
name: nils-cli-docs-plan-cleanup
description: Prune outdated docs/plans and reconcile related docs safely
---

# Docs Plan Cleanup

## Contract

Prereqs:

- Run inside the `nils-cli` git work tree (the script resolves repo root via `git`).
- `bash`, `git`, `find`, and `rg` available on `PATH`.
- `docs/plans/` exists and contains markdown plans to evaluate.

Inputs:

- Optional:
  - `--project-path <path>`: target project path (default current directory).
  - `--keep-plan <path|name>`: plan to preserve (repeatable). Supports:
    - repo-relative path (for example `docs/plans/foo-plan.md`),
    - filename (`foo-plan.md`),
    - stem (`foo-plan`).
  - `--keep-plans-file <path>`: newline list of plans to preserve (`#` comments allowed).
  - `--execute`: apply deletions (default is dry-run).
  - `--delete-important`: also delete `docs/specs/**` and `docs/runbooks/**` files that are only tied to removed plans.
  - `--delete-empty-dirs`: remove empty directories under `docs/` after deletion.

Outputs:

- Dry-run summary with stable report template `docs-plan-cleanup-report:v1`:
  - `[plan_md_to_clean]`: plans that will be removed.
  - `[plan_related_md_to_clean]`: non-plan docs that only depend on removed plans and are safe to delete.
  - `[plan_related_md_kept_referenced_elsewhere]`: plan-related docs that are **not deleted** because non-plan markdown files still reference them.
  - `[plan_related_md_to_rehome]`: important docs (`docs/specs/**`, `docs/runbooks/**`) to consolidate before deletion.
  - `[non_docs_md_referencing_removed_plan]`: markdown outside `docs/` that still references removed plans.
- In `--execute` mode:
  - deletes all non-kept `docs/plans/**/*.md`,
  - deletes related `docs/**/*.md` files that only reference removed plans and are not externally referenced,
  - keeps important spec/runbook docs unless `--delete-important` is set.

Exit codes:

- `0`: success
- `1`: runtime failure
- `2`: usage error or invalid keep-plan input

Failure modes:

- Not running inside a git work tree.
- `docs/plans/` missing.
- Required tool missing (`git`, `rg`, `find`).
- `--keep-plan` / `--keep-plans-file` references unknown or ambiguous plans.
- Deletion fails due to filesystem permissions or path conflicts.

## Scripts (only entrypoints)

- `<PROJECT_ROOT>/.agents/skills/nils-cli-docs-plan-cleanup/scripts/nils-cli-docs-plan-cleanup.sh`

## Workflow

1. Ask the user which plans must be preserved.
2. Run dry-run first to review impact:
   - `.agents/skills/nils-cli-docs-plan-cleanup/scripts/nils-cli-docs-plan-cleanup.sh --keep-plan <plan>`
3. Review dry-run output:
   - `plan_related_md_kept_referenced_elsewhere` means those docs are protected from auto-delete.
   - `plan_related_md_to_rehome` should be moved/summarized into canonical docs:
     - workspace rules -> `docs/specs/` or root docs (`README.md`, `DEVELOPMENT.md`),
     - crate-local rules -> `crates/<crate>/docs/`.
4. Apply cleanup only after review:
   - `.agents/skills/nils-cli-docs-plan-cleanup/scripts/nils-cli-docs-plan-cleanup.sh --keep-plan <plan> --execute --delete-empty-dirs`
5. If strong confidence exists that flagged important docs are obsolete, add `--delete-important` explicitly.
