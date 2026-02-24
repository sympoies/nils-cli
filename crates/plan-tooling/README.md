# plan-tooling

## Overview
plan-tooling works with Plan Format v1 markdown files. It can parse plans to JSON, validate plan
files, compute dependency batches for a sprint, scaffold new plans, and generate deterministic
task-to-PR split specs.

## Usage
```text
Usage:
  plan-tooling <command> [args]

Commands:
  to-json   Parse a plan markdown file into a stable JSON schema
  validate  Lint plan markdown files
  batches   Compute dependency layers (parallel batches) for a sprint
  split-prs Build deterministic task-to-PR split records
  scaffold  Create a new plan from template
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
- `split-prs --file <plan.md> --scope <plan|sprint> [--sprint <n>] --pr-grouping <per-sprint|group> [--pr-group <task-or-plan-id>=<group>]... [--strategy deterministic|auto] [--format json|tsv]`
- `--strategy auto` is reserved for future scoring based on `Complexity`, `Location`, and
  `Dependencies`; in v1 it intentionally returns `not implemented`.
- deterministic examples:
  - `split-prs --file docs/plans/example-plan.md --scope sprint --sprint 1 --pr-grouping per-sprint --format tsv`
  - `split-prs --file docs/plans/example-plan.md --scope sprint --sprint 2 --pr-grouping group --pr-group S2T1=isolated --pr-group S2T2=shared --pr-group S2T3=shared --format json`

### scaffold
- `scaffold --slug <kebab-case> [--title <title>] [--force]`: Write to
  `docs/plans/<slug>-plan.md` (or `<slug>.md` if the slug already ends with `-plan`).
- `scaffold --file <path> [--title <title>] [--force]`: Write to a specific `-plan.md` path.

## Template
- Plan template: `crates/plan-tooling/plan-template.md`.

## Exit codes
- `0`: success and help output.
- `1`: validation or runtime errors.
- `2`: usage errors.

## Docs

- [Docs index](docs/README.md)
- [Migration runbook](docs/runbooks/split-prs-migration.md)
