# plan-tooling

## Overview
plan-tooling works with Plan Format v1 markdown files. It can parse plans to JSON, validate plan
files, compute dependency batches for a sprint, scaffold new plans, and generate task-to-PR split
specs in deterministic or auto strategy modes.

## Usage
```text
Usage:
  plan-tooling <command> [args]

Commands:
  to-json   Parse a plan markdown file into a stable JSON schema
  validate  Lint plan markdown files
  batches   Compute dependency layers (parallel batches) for a sprint
  split-prs Build task-to-PR split records (deterministic/auto)
  scaffold  Create a new plan from template
  completion Export shell completion script
  help      Display help message

Help:
  plan-tooling help
  plan-tooling --help
```

## Commands

### to-json
- `to-json --file <plan.md> [--sprint <n>] [--pretty]`: Parse a plan file and output JSON.

### validate
- `validate [--file <path>]... [--format text|json]`: Validate plan files. With no `--file`, scans
  tracked `docs/plans/*-plan.md` files.

### batches
- `batches --file <plan.md> --sprint <n> [--format json|text]`: Compute dependency batches for a
  sprint.

### split-prs
- `split-prs --file <plan.md> --scope <plan|sprint> [--sprint <n>] --pr-grouping <per-sprint|group> [--pr-group <task-or-plan-id>=<group>]... [--strategy deterministic|auto] [--explain] [--owner-prefix <text>] [--branch-prefix <text>] [--worktree-prefix <text>] [--format json|tsv]`
- value options accept both `--key value` and `--key=value`.
- deterministic mode:
  - `--pr-grouping per-sprint`: one shared `pr_group` per sprint (`s<n>`).
  - `--pr-grouping group`: pass `--pr-group` for every selected task.
- auto mode:
  - scoring inputs are `Complexity`, dependency topology, and `Location` overlap.
  - in `pr-grouping=group`, `--pr-group` mappings are optional pins; remaining tasks are auto-grouped.
  - when sprint metadata provides `Execution Profile` parallel width hints, auto grouping targets that lane count (deterministic fallback merges apply when needed).
  - `pr-grouping=per-sprint` keeps one shared group per sprint (`s<n>`).
  - ordering and tie-breakers stay deterministic (`Task N.M`, then `SxTy`, then lexical summary).
  - emitted lane metadata (`pr_group`, anchor notes, prefixes) is consumed by `plan-issue` runtime-truth validation and sprint artifact rendering.
- deterministic examples:
  - `split-prs --file docs/plans/example-plan.md --scope sprint --sprint 1 --pr-grouping per-sprint --format tsv`
  - `split-prs --file docs/plans/example-plan.md --scope sprint --sprint 2 --pr-grouping group --pr-group S2T1=isolated --pr-group S2T2=shared --pr-group S2T3=shared --format json`
- auto example:
  - `split-prs --file docs/plans/example-plan.md --scope sprint --sprint 2 --pr-grouping group --strategy auto --format json`
- rollback switchback:
  - if auto rollout is unhealthy, pin orchestration calls to `--strategy deterministic` until follow-up fixes land.

### scaffold
- `scaffold --slug <kebab-case> [--title <title>] [--force]`: Write to
  `docs/plans/<slug>-plan.md` (or `<slug>.md` if the slug already ends with `-plan`).
- `scaffold --file <path> [--title <title>] [--force]`: Write to a specific `-plan.md` path.

### completion
- `completion <bash|zsh>`: Export completion script for shell integration.

## Quick examples
```bash
# Parse one plan to JSON
plan-tooling to-json --file docs/plans/example-plan.md --pretty

# Validate all tracked plan docs (default discovery)
plan-tooling validate

# Compute sprint batches in text mode
plan-tooling batches --file docs/plans/example-plan.md --sprint 2 --format text

# Split sprint tasks with deterministic groups
plan-tooling split-prs \
  --file docs/plans/example-plan.md \
  --scope sprint \
  --sprint 2 \
  --pr-grouping group \
  --pr-group S2T1=isolated \
  --pr-group S2T2=shared \
  --strategy deterministic \
  --format json

# Export completion
plan-tooling completion zsh > completions/zsh/_plan-tooling
```

## Template
- Plan template: `crates/plan-tooling/plan-template.md`.

## Exit codes
- `0`: success and help output.
- `1`: validation or runtime errors.
- `2`: usage errors.

## Docs

- [Docs index](docs/README.md)
- [Migration runbook](docs/runbooks/split-prs-migration.md)
