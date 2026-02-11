---
name: publish-crates-io
description: Publish one, many, or all workspace crates to crates.io with rate-limit-aware retry and reporting.
---

# Publish Crates IO

## Contract

Prereqs:

- Run inside the `nils-cli` git work tree.
- `bash`, `git`, `python3`, and `cargo` available on `PATH`.
- For real publish mode:
  - local flow: `cargo login` completed, or
  - CI flow: `CARGO_REGISTRY_TOKEN` provided by workflow secrets.
- Workspace publish order file exists:
  - `release/crates-io-publish-order.txt`
- crates.io status helper exists for post-run verification:
  - `scripts/crates-io-status.sh`

Inputs:

- Crate selection (one of):
  - `--crate <name>` (repeatable),
  - `--crates "a b,c"`,
  - `--all` (or omit selectors to use default list file).
- Optional behavior flags:
  - `--wait-retry`: rate-limited publish waits and retries until all uploads complete.
  - default (no `--wait-retry`): stop on first rate-limit error and report next eligible upload time.
  - `--max-retries <N>`: retry cap per crate in wait mode (`0` = unlimited).
  - `--default-retry-seconds <N>`: fallback wait if retry hint is missing.
  - `--dry-run-only`: validate publishability without uploading.
  - `--report-file <path>`: custom report output path.
  - `--skip-status-check`: skip post-run crates.io snapshot generation.

Outputs:

- Attempts publish in selected order with dry-run before each upload.
- Produces a markdown report with:
  - summary counts (published/skipped/failed/pending),
  - next eligible publish time when rate-limited,
  - per-crate record including crate name, version, status, start/end time, attempts, notes,
  - crates.io status snapshot metadata (json/text path + status).
- Default report path:
  - `$CODEX_HOME/out/crates-io-publish-report-<timestamp>.md`
- Default status snapshot outputs:
  - `${report_file%.md}.status.json`
  - `${report_file%.md}.status.md`

Exit codes:

- `0`: all selected crates completed successfully (or dry-run passed)
- `1`: publish/dry-run failure, halted due rate limit, or pending crates remain
- `2`: usage error

Failure modes:

- Crate not found in workspace metadata.
- Selected crate has `publish = false`.
- Publish order invalid for workspace path dependencies.
- Dirty worktree in publish mode without `--allow-dirty`.
- `cargo publish` fails (non-rate-limit error).
- Rate-limit encountered:
  - default mode: stop and report retry time,
  - wait mode: retry until success or `--max-retries` reached.

## Scripts (only entrypoints)

- `<PROJECT_ROOT>/.codex/skills/publish-crates-io/scripts/publish-crates-io.sh`

## Workflow

1. Resolve crate set (single / multi / all) and validate against workspace metadata.
2. For each selected crate, run `cargo publish --dry-run` first.
3. In publish mode:
   - publish sequentially in the selected order,
   - skip already-published versions on crates.io by default.
4. Handle rate-limit conditions:
   - default mode: stop immediately and report next eligible publish time,
   - `--wait-retry`: sleep/retry and continue until all crates are done.
5. Run post-run crates.io snapshot using:
   - `scripts/crates-io-status.sh` (with `--fail-on-missing` when publish run is otherwise successful).
6. Write a structured report from template:
   - `.codex/skills/publish-crates-io/references/PUBLISH_REPORT_TEMPLATE.md`
