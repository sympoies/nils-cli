# image-processing parity spec

## Overview
`image-processing` is a batch image transformation CLI used by the Codex `image-processing` skill.
It is a Rust port of the existing Python implementation under:
`https://github.com/graysurf/codex-kit/blob/main/skills/tools/media/image-processing/scripts/image_processing.py`.

The CLI shells out to external tools (ImageMagick; optional JPEG/WebP encoders) and focuses on:
- safe output-mode gating (no accidental overwrites; explicit in-place confirmations)
- deterministic, auditable runs via `--json` / `--report`

## Entry point
- Command: `image-processing <subcommand> [flags]`
- Exit codes:
  - `0`: success (all items ok)
  - `1`: runtime failure (missing required tools or one-or-more items failed)
  - `2`: usage error (invalid flags/values/output modes)

## Subcommands
- `info`
- `auto-orient`
- `convert`
- `resize`
- `rotate`
- `crop`
- `pad`
- `flip`
- `flop`
- `optimize`

## Common flags
Inputs:
- `--in <path>` (repeatable; file or directory)
- `--recursive` (recurse into input directories)
- `--glob <pattern>` (repeatable; filters directory candidates by filename glob, e.g. `*.png`)

Output mode (required for output-producing subcommands; forbidden for `info`):
- exactly one of:
  - `--out <file>` (single input only)
  - `--out-dir <dir>` (batch mode)
  - `--in-place` (destructive; requires `--yes`)
- `--yes` (required with `--in-place`)
- `--overwrite` (allow overwriting existing outputs; default is to refuse)

Reproducibility:
- `--dry-run` (do not execute transforms; do not write output images)
- `--json` (stdout is JSON summary only; also writes `summary.json` under `out/`)
- `--report` (writes `report.md` under `out/` and includes `report_path` in the JSON summary)

Shared behavior:
- Auto orientation is enabled by default for output-producing subcommands; disable with `--no-auto-orient`.
- `--strip-metadata` removes metadata (EXIF/XMP/ICC) when supported by the backend.
- `--background <color>` is required for some operations when writing non-alpha formats (e.g. JPEG).

## External dependencies
Required (missing → stderr `image-processing: error: ...`, exit 1):
- ImageMagick:
  - preferred: `magick`
  - fallback: `convert` + `identify`

Optional (used when present; otherwise fallback to ImageMagick backend):
- JPEG optimize: `djpeg` + `cjpeg`
- WebP optimize: `dwebp` + `cwebp`

Optional (fallback):
- `git` is used only to detect repo root for run artifacts; if missing/fails, runs are rooted at `cwd`.

## Run artifacts
When `--json` or `--report` is provided, a run directory is created:

- `out/image-processing/runs/<run_id>/summary.json`
- `out/image-processing/runs/<run_id>/report.md` (only when `--report`)

`<run_id>` format: `<UTC YYYYmmdd-HHMMSS>-<6 hex>`.

## JSON summary schema (schema_version = 1)
Top-level keys:
- `schema_version` (int)
- `run_id` (string|null)
- `cwd` (string)
- `operation` (string; subcommand name)
- `backend` (string; `imagemagick:magick` or `imagemagick:convert`)
- `report_path` (string|null)
- `dry_run` (bool)
- `options` (object)
- `commands` (array of strings; shell-escaped command lines)
- `collisions` (array; present even if empty)
- `skipped` (array; present even if empty)
- `warnings` (array; present even if empty)
- `items` (array of per-input results)

Per-item keys:
- `input_path` (string)
- `output_path` (string|null)
- `status` (`ok`|`error`)
- `input_info` (object)
- `output_info` (object|null)
- `commands` (array of strings)
- `warnings` (array of strings)
- `error` (string|null)

## Output text contract
- With `--json`: stdout is a single JSON object plus newline.
- Without `--json`: stdout is a human summary:
  - `operation: <subcommand>`
  - optional: `run_dir: <path>` when `--json` or `--report` is used
  - then one `- <status>: <input> -> <output>` line per item

# image-processing fixtures

## Backend detection
- Missing backend:
  - Setup: PATH without `magick`, `convert`, and `identify`
  - Command: `image-processing info --in a.png --json`
  - Expect: exit `1`, stderr contains `missing ImageMagick (need \`magick\` or both \`convert\` + \`identify\`)`

## Output mode gating
- Missing output mode (output-producing subcommand):
  - Command: `image-processing convert --in a.png --to webp --json`
  - Expect: exit `2`, error mentions `must specify exactly one output mode`
- Multiple output modes:
  - Command: `image-processing resize --in a.png --scale 2 --out x.png --out-dir out --json`
  - Expect: exit `2`
- In-place requires confirmation:
  - Command: `image-processing rotate --in a.png --degrees 90 --in-place --json`
  - Expect: exit `2`, error mentions `--in-place is destructive and requires --yes`

## info
- Command: `image-processing info --in dir --recursive --glob '*.png' --json`
- Expect: JSON `items` length matches resolved inputs; each item `output_path` is `null`.

## auto-orient
- Command: `image-processing auto-orient --in a.jpg --out out/a.jpg --json`
- Expect: output exists; JSON item `status=ok`.

## convert
- Basic:
  - Command: `image-processing convert --in a.png --to webp --out out/a.webp --json`
  - Expect: output ext matches `--to`; JSON item ok.
- Alpha → JPEG requires background (usage error):
  - Command: `image-processing convert --in alpha.png --to jpg --out out/a.jpg --json`
  - Expect: exit `2`, error mentions `alpha input cannot be converted to JPEG without a background`

## resize
- Scale:
  - Command: `image-processing resize --in a.png --scale 2 --out out/a.png --json`
  - Expect: command list includes `-resize 200%` (unless `--no-pre-upscale`).
- Box fit contain requires background for JPEG (runtime item error):
  - Command: `image-processing resize --in a.jpg --width 10 --height 10 --fit contain --out out/a.jpg --json`
  - Expect: exit `1`, item `status=error`, `error` mentions `contain fit requires padding background`

## rotate
- Missing degrees (usage):
  - Command: `image-processing rotate --in a.png --out out/a.png --json`
  - Expect: exit `2`, error mentions `rotate requires --degrees`
- Non-right-angle JPEG requires background (runtime item error):
  - Command: `image-processing rotate --in a.jpg --degrees 13 --out out/a.jpg --json`
  - Expect: exit `1`, item error mentions background requirement.

## crop
- Exactly one mode required (usage):
  - Command: `image-processing crop --in a.png --out out/a.png --json`
  - Expect: exit `2`
- Aspect crop:
  - Command: `image-processing crop --in a.png --aspect 1:1 --out out/a.png --json`
  - Expect: ok.

## pad
- Requires width+height (usage):
  - Command: `image-processing pad --in a.png --out out/a.png --json`
  - Expect: exit `2`

## flip / flop
- Command: `image-processing flip --in a.png --out out/a.png --json`
- Expect: ok.

## optimize
- JPG path:
  - Command: `image-processing optimize --in a.jpg --out out/a.jpg --json`
  - Expect: ok; when `cjpeg`/`djpeg` are present on PATH, commands include a `djpeg | cjpeg` pipeline.
- WEBP path:
  - Command: `image-processing optimize --in a.webp --out out/a.webp --json`
  - Expect: ok; when `cwebp`/`dwebp` are present on PATH, commands include both decode + encode steps.

