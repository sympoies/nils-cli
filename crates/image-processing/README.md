# image-processing

## Overview
image-processing is a batch image CLI with two execution paths:
- Rust-backed generation (`generate`) for deterministic preset icons (`png|webp|svg`) with no
  ImageMagick requirement.
- Legacy transform subcommands (`auto-orient|convert|resize|rotate|crop|pad|flip|flop|optimize`)
  that execute through ImageMagick.

All output-producing subcommands require exactly one output mode (`--out`, `--out-dir`, or
`--in-place` with `--yes`), with extra safety rules for `generate`.

## Usage
```text
Usage:
  image-processing <subcommand> [flags]

Subcommands:
  info | generate | auto-orient | convert | resize | rotate | crop | pad | flip | flop | optimize

Help:
  image-processing --help
```

## Commands
- `info`: Probe inputs and emit metadata summary (no output mode).
- `generate`: Deterministic preset icon generation contract scaffold.
  - Presets: `--preset info|success|warning|error|help` (repeatable).
  - Colors/styles: `--fg`, `--bg`, `--stroke`, `--stroke-width`, `--padding`.
  - Size/format: `--size <px>`, `--to png|webp|svg`.
  - Safety: no `--in`, no `--in-place`; output mode must be `--out` or `--out-dir`.
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
- Inputs (legacy transform commands only): `--in <path>` (repeatable, required), `--recursive`,
  `--glob <pattern>` (repeatable)
- Output mode: `--out <file>`, `--out-dir <dir>`, or `--in-place` (requires `--yes`)
- Output controls: `--overwrite`, `--dry-run`, `--json`, `--report`
- Transform options: `--no-auto-orient`, `--strip-metadata`, `--background <color>`

## `generate` deterministic contract
- Deterministic defaults:
  - `--to`: `png`
  - `--size`: `64`
  - `--fg`: `#ffffff`
  - `--bg`: `#0f62fe`
  - `--stroke`: unset (no stroke)
  - `--stroke-width`: `0`
  - `--padding`: `0`
- Validation matrix scaffold:
  - Required: `--preset` for `generate`.
  - Forbidden: `--in`, `--recursive`, `--glob`, `--in-place`.
  - Output mode:
    - Single variant (`1` resolved preset) => `--out` required.
    - Multi variant (`>1` presets) => `--out-dir` required.
  - `--to` accepts only `png|webp|svg` (default `png`).
- `--out-dir` output naming:
  - Variant filename format:
    - `<preset>__size-<size>__fg-<fg>__bg-<bg>__stroke-<stroke-or-none>__sw-<stroke-width>__pad-<padding>.<to>`
  - Normalization:
    - Colors are lowercase, keep hex digits only (drop `#`).
    - Decimal values use compact canonical form (`2`, `1.5`, `0`).
    - Missing stroke uses literal token `none`.
  - Example:
    - `warning__size-64__fg-111111__bg-ffd166__stroke-none__sw-0__pad-0.png`

## `generate` safety constraints
- `generate` is input-free:
  - Forbidden flags: `--in`, `--recursive`, `--glob`, `--in-place`.
- Output mode rules are strict:
  - Single preset result requires `--out`.
  - Multiple preset results require `--out-dir`.
- Existing output files are protected by default:
  - Pass `--overwrite` to replace an existing output file.

## `generate` runnable examples (`png`, `webp`, `svg`)
```bash
mkdir -p out/plan-doc-examples
```

```bash
cargo run -p nils-image-processing -- generate \
  --preset info \
  --size 32 \
  --fg '#ffffff' \
  --bg '#0f62fe' \
  --to png \
  --out out/plan-doc-examples/info.png \
  --overwrite \
  --json
```

```bash
cargo run -p nils-image-processing -- generate \
  --preset warning \
  --size 32 \
  --fg '#111111' \
  --bg '#ffd166' \
  --to webp \
  --out out/plan-doc-examples/warning.webp \
  --overwrite \
  --json
```

```bash
cargo run -p nils-image-processing -- generate \
  --preset help \
  --size 32 \
  --fg '#ffffff' \
  --bg '#3a86ff' \
  --to svg \
  --out out/plan-doc-examples/help.svg \
  --overwrite \
  --json
```

## Exit codes
- `0`: Success with no item errors.
- `1`: Runtime failure or one-or-more items failed.
- `2`: Usage/validation error.

## Dependencies
- `generate`: no external binary dependency (Rust backend).
- Legacy transform subcommands: ImageMagick (`magick`, or `convert` + `identify`).
- Optional: `cjpeg`/`djpeg` for JPEG optimize, `cwebp`/`dwebp` for WebP optimize.
