# image-processing

## Overview
image-processing is a batch image transformation CLI backed by ImageMagick. It supports
convert/resize/rotate/crop/pad/flip/flop/optimize workflows plus JSON/report output for
auditability. All output-producing subcommands require exactly one output mode (`--out`,
`--out-dir`, or `--in-place` with `--yes`).

## Usage
```text
Usage:
  image-processing <subcommand> [flags]

Subcommands:
  info | auto-orient | convert | resize | rotate | crop | pad | flip | flop | optimize

Help:
  image-processing --help
```

## Commands
- `info`: Probe inputs and emit metadata summary (no output mode).
- `auto-orient`: Auto-orient images and write outputs.
- `convert`: Convert to a target format; requires `--to png|jpg|webp`.
- `resize`: Resize by `--scale`, `--width`/`--height`, or `--aspect` + `--fit` (`contain|cover|stretch`).
- `rotate`: Rotate by degrees; requires `--degrees`.
- `crop`: Crop by `--rect`, `--size`, or `--aspect` (exactly one).
- `pad`: Pad to a target size; requires `--width` and `--height`.
- `flip`: Apply ImageMagick `-flip`.
- `flop`: Apply ImageMagick `-flop`.
- `optimize`: Optimize `jpg` or `webp` outputs; supports `--quality`, `--lossless`, `--no-progressive`.

## Common flags
- Inputs: `--in <path>` (repeatable, required), `--recursive`, `--glob <pattern>` (repeatable)
- Output mode: `--out <file>`, `--out-dir <dir>`, or `--in-place` (requires `--yes`)
- Output controls: `--overwrite`, `--dry-run`, `--json`, `--report`
- Transform options: `--no-auto-orient`, `--strip-metadata`, `--background <color>`

## Exit codes
- `0`: Success with no item errors.
- `1`: Runtime failure or one-or-more items failed.
- `2`: Usage/validation error.

## Dependencies
- Required: ImageMagick (`magick`, or `convert` + `identify`).
- Optional: `cjpeg`/`djpeg` for JPEG optimize, `cwebp`/`dwebp` for WebP optimize.
