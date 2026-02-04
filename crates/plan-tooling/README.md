# plan-tooling

## Overview
`plan-tooling` is a small CLI for working with Plan Format v1 markdown files (typically
`docs/plans/*-plan.md`) in this repository.

It is used to:
- parse plans into a stable JSON schema (`to-json`)
- lint/validate plans (`validate`)
- compute parallelizable task layers for a sprint (`batches`)
- scaffold new plans from a shared template (`scaffold`)

## Commands
- `plan-tooling to-json ...`
- `plan-tooling validate ...`
- `plan-tooling batches ...`
- `plan-tooling scaffold ...`

## Examples
```bash
plan-tooling validate
plan-tooling validate --format json | jq .
plan-tooling to-json --file docs/plans/plan-tooling-cli-consolidation-plan.md --pretty | jq .
plan-tooling batches --file docs/plans/plan-tooling-cli-consolidation-plan.md --sprint 1 --format text
plan-tooling scaffold --slug my-new-cli --title "My new CLI plan"
```

To run from the workspace without installing:
```bash
cargo run -p plan-tooling -- validate
```

## Template
The plan template embedded into the binary lives at:
- `crates/plan-tooling/plan-template.md`

