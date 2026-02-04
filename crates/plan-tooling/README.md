# plan-tooling

## Overview
plan-tooling works with Plan Format v1 markdown files. It can parse plans to JSON, validate plan
files, compute dependency batches for a sprint, and scaffold new plans.

## Usage
```text
Usage:
  plan-tooling <command> [args]

Commands:
  to-json   Parse a plan markdown file into a stable JSON schema
  validate  Lint plan markdown files
  batches   Compute dependency layers (parallel batches) for a sprint
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
