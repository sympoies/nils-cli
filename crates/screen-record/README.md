# screen-record

## Overview
screen-record is a macOS 12+ CLI that records a single window to a video file using ScreenCaptureKit
and AVFoundation. It also exposes parseable window/app lists to make selection deterministic in
scripts.

## Usage
```text
screen-record [options]
```

## Flags
| Flag | Value | Default | Description |
| --- | --- | --- | --- |
| `--list-windows` | (none) | (none) | Print selectable windows as TSV and exit. |
| `--list-apps` | (none) | (none) | Print selectable apps as TSV and exit. |
| `--window-id` | `<id>` | (none) | Record a specific window id. |
| `--app` | `<name>` | (none) | Select a window by app/owner name (case-insensitive substring). |
| `--window-name` | `<name>` | (none) | Narrow `--app` selection by window title substring. |
| `--active-window` | (none) | (none) | Record the frontmost window on the current Space. |
| `--duration` | `<seconds>` | (required for recording) | Record for N seconds. |
| `--audio` | `off\|system\|mic\|both` | `off` | Control audio capture. `both` requires `.mov`. |
| `--path` | `<path>` | (required for recording) | Output file path. |
| `--format` | `mov\|mp4` | (auto) | Explicit container selection. Overrides extension. |
| `--preflight` | (none) | (none) | Check Screen Recording permission and exit. |
| `--request-permission` | (none) | (none) | Best-effort permission request + status check, then exit. |
| `-h, --help` | (none) | (none) | Show help. |
| `-V, --version` | (none) | (none) | Show version. |

## Mode rules
- Exactly one mode must be selected: `--list-windows`, `--list-apps`, `--preflight`,
  `--request-permission`, or recording.
- Recording mode requires exactly one selector: `--window-id`, `--active-window`, or `--app`.
- `--window-name` is only valid together with `--app`.
- `--duration` is required for recording mode.

## Output contract
- Success (recording): stdout prints only the resolved output file path followed by `\n`.
- Success (list): stdout prints only TSV rows followed by `\n`.
- Success (preflight/request): stdout is empty; any user messaging goes to stderr.
- Errors: stdout is empty; stderr contains user-facing errors (no stack traces).

## List output (TSV)
All list output is UTF-8 TSV with no header and one record per line. Tabs or newlines in string
fields are normalized to a single space. Sorting is deterministic.

### `--list-windows` column order
1. `window_id` (decimal)
2. `owner_name`
3. `window_title` (empty when missing)
4. `x` (decimal)
5. `y` (decimal)
6. `width` (decimal)
7. `height` (decimal)
8. `on_screen` (`true` or `false`)

Sorting: by `owner_name`, then `window_title`, then `window_id`.

### `--list-apps` column order
1. `app_name`
2. `pid` (decimal)
3. `bundle_id` (empty when missing)

Sorting: by `app_name`, then `pid`.

## Selection rules
- `--window-id <id>` selects exactly that window id.
- `--active-window` selects the single frontmost window on the current Space.
- `--app <name>` matches windows by owner/app name substring (case-insensitive).
- `--window-name <name>` further filters by title substring (case-insensitive).
- If multiple windows remain after filtering, and no single frontmost window can be chosen,
  selection is ambiguous and the CLI exits 2 with candidate output.

## Ambiguous selection errors
Ambiguous selection is a usage error (exit 2). The error format is fixed:

```text
error: multiple windows match --app "<app>"
error: refine with --window-name or use --window-id
<window_id>\t<owner_name>\t<window_title>\t<x>\t<y>\t<width>\t<height>\t<on_screen>
<window_id>\t<owner_name>\t<window_title>\t<x>\t<y>\t<width>\t<height>\t<on_screen>
```

Candidate rows are identical to `--list-windows` TSV output and are printed to stderr.

## Container selection (.mov vs .mp4)
- If `--format` is provided, its value selects the container.
- Otherwise, `.mov` or `.mp4` is selected from the `--path` extension.
- If no supported extension is present, the container defaults to `.mov`.
- If `--format` conflicts with the `--path` extension, exit 2 with a usage error.
- `--audio both` requires `.mov`; using `.mp4` exits 2 with a clear error.

## Exit codes
- `0`: Success (recording completed, list output printed, or preflight success).
- `1`: Runtime failure (permission denied, capture/encode failure).
- `2`: Usage error (invalid flags, ambiguous selection, invalid format).

## Environment
- `CODEX_SCREEN_RECORD_TEST_MODE=1`: Use deterministic fixtures instead of macOS APIs. In test
  mode, recording copies a fixture file matching the selected container.

## Examples
List windows:
```bash
screen-record --list-windows
```

List apps:
```bash
screen-record --list-apps
```

Record by app name:
```bash
screen-record --app Terminal --duration 3 --audio off --path "./recordings/terminal.mov"
```

Record by window id:
```bash
screen-record --window-id 4811 --duration 5 --audio off --path "./recordings/window-4811.mov"
```

Record for a short duration:
```bash
screen-record --active-window --duration 2 --audio off --path "./recordings/active.mov"
```

Record with system audio:
```bash
screen-record --app Terminal --duration 5 --audio system --path "./recordings/terminal-audio.mov"
```
