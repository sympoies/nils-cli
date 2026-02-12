# image-processing

## Overview
`image-processing` is a batch image CLI with two execution paths:
- Rust SVG path:
  - `convert --from-svg <path> --to png|webp|svg --out <file>`
  - `svg-validate --in <svg> --out <svg>`
- Legacy transform path (ImageMagick):
  - `auto-orient|convert|resize|rotate|crop|pad|flip|flop|optimize`

`generate` has been removed. Use intent -> SVG -> `svg-validate` -> `convert --from-svg` instead.

## Usage
```text
Usage:
  image-processing <subcommand> [flags]

Subcommands:
  info | svg-validate | auto-orient | convert | resize | rotate | crop | pad | flip | flop | optimize

Help:
  image-processing --help
```

## Commands
- `info`: Probe inputs and emit metadata summary (no output mode).
- `svg-validate`: Validate + sanitize one SVG input into one SVG output.
- `convert`: Convert image formats.
  - Legacy mode: `--in ... --to png|jpg|webp`.
  - SVG mode: `--from-svg <path> --to png|webp|svg --out <file>`.
- `resize`: Resize by `--scale`, `--width`/`--height`, or `--aspect` + `--fit` (`contain|cover|stretch`).
- `rotate`: Rotate by degrees; requires `--degrees`.
- `crop`: Crop by `--rect`, `--size`, or `--aspect` (exactly one).
- `pad`: Pad to a target size; requires `--width` and `--height`.
- `flip`: Apply ImageMagick `-flip`.
- `flop`: Apply ImageMagick `-flop`.
- `optimize`: Optimize `jpg` or `webp` outputs; supports `--quality`, `--lossless`, `--no-progressive`.

## Common flags
- Inputs (legacy transform commands only): `--in <path>` (repeatable, required), `--recursive`, `--glob <pattern>` (repeatable)
- Source SVG mode: `--from-svg <path>` (convert only)
- Output mode: `--out <file>`, `--out-dir <dir>`, or `--in-place` (requires `--yes`)
- Output controls: `--overwrite`, `--dry-run`, `--json`, `--report`
- Transform options: `--no-auto-orient`, `--strip-metadata`, `--background <color>`

## `--from-svg` contract (v1)
- Allowed only on `convert`.
- Required: `--to png|webp|svg`, `--out <file>`.
- Forbidden with `--from-svg`: `--in`, `--recursive`, `--glob`, `--out-dir`, `--in-place`.
- `--out` extension must match `--to`.
- This path is Rust-backed (`usvg`/`resvg`) and does not require ImageMagick.

## `svg-validate` contract
- Required: exactly one `--in <svg>` and `--out <svg>`.
- Forbidden: `--from-svg`, `--recursive`, `--glob`, `--out-dir`, `--in-place`.
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
  --json
```

## Exit codes
- `0`: Success with no item errors.
- `1`: Runtime failure or one-or-more items failed.
- `2`: Usage/validation error.

## Dependencies
- `convert --from-svg` and `svg-validate`: no external binary dependency (Rust backend).
- Legacy transform subcommands: ImageMagick (`magick`, or `convert` + `identify`).
- Optional: `cjpeg`/`djpeg` for JPEG optimize, `cwebp`/`dwebp` for WebP optimize.
