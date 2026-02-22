# image-processing

## Overview
`image-processing` provides a modern SVG-first flow:
- `svg-validate --in <svg> --out <svg>`
- `convert --from-svg <path> --to png|webp|svg --out <file>`

`generate` is removed.

## Usage
```text
Usage:
  image-processing <subcommand> [flags]

Subcommands:
  convert | svg-validate

Help:
  image-processing --help
```

## Commands
- `svg-validate`: Validate and sanitize one SVG input into one SVG output.
- `convert`: Render trusted SVG source into `png`, `webp`, or `svg` output.

## Common flags
- Input:
  - `svg-validate`: `--in <path>` (exactly one)
  - `convert`: `--from-svg <path>`
- Output: `--out <file>`
- Output controls: `--overwrite`, `--dry-run`, `--json`, `--report`
- Render sizing for raster output: `--width`, `--height`

## `convert --from-svg` contract (v1)
- Required: `--from-svg`, `--to png|webp|svg`, `--out <file>`.
- Forbidden: `--in`.
- `--out` extension must match `--to`.
- Optional: `--width` and `--height` for `png`/`webp` sizing.
- `--to svg` does not support `--width`/`--height`.

## `svg-validate` contract
- Required: exactly one `--in <svg>` and `--out <svg>`.
- Forbidden: `--from-svg`, `--to`, `--width`, `--height`.
- Output is deterministic for identical input.

## Examples
```bash
mkdir -p out/plan-doc-examples
```

```bash
cargo run -p nils-image-processing -- svg-validate \
  --in crates/image-processing/tests/fixtures/llm-svg-valid.svg \
  --out out/plan-doc-examples/llm.cleaned.svg
```

```bash
cargo run -p nils-image-processing -- convert \
  --from-svg out/plan-doc-examples/llm.cleaned.svg \
  --to png \
  --out out/plan-doc-examples/llm.png \
  --json
```

```bash
cargo run -p nils-image-processing -- convert \
  --from-svg crates/image-processing/tests/fixtures/sample-icon.svg \
  --to webp \
  --out out/plan-doc-examples/sample.webp \
  --width 512 \
  --json
```

## Exit codes
- `0`: Success with no item errors.
- `1`: Runtime failure or one-or-more items failed.
- `2`: Usage/validation error.

## Dependencies
- `convert --from-svg` and `svg-validate`: no external binary dependency (Rust backend).

## Docs

- [Docs index](docs/README.md)
- [LLM SVG workflow runbook](docs/runbooks/llm-svg-workflow.md)
