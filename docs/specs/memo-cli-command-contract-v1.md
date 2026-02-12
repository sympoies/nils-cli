# memo-cli Command Contract v1

## Purpose
This document defines the command-level behavior contract for `memo-cli` MVP commands:
`add`, `list`, `search`, `report`, `fetch`, and `apply`.

It aligns with:
- `docs/runbooks/new-cli-crate-development-standard.md`
- `docs/specs/cli-service-json-contract-guideline-v1.md`

## Scope
- Human-readable output is the default mode.
- JSON output is opt-in via `--json` or `--format json`.
- Raw memo capture is append-only; agent enrichment is a derived layer.

## Command Surface

```text
Usage:
  memo-cli [--db <path>] [--format <text|json> | --json] <command> [options]

Commands:
  add <text>
  list
  search <query>
  report <week|month>
  fetch
  apply
```

### Shared flags
- `--db <path>`: SQLite file path.
  Default: `$XDG_DATA_HOME/nils-cli/memo.db` or `~/.local/share/nils-cli/memo.db`.
- `--format <text|json>`: output mode selector. Default: `text`.
- `--json`: shorthand for `--format json`.

### `--json` and `--format json` rules
- `--json` and `--format json` are equivalent.
- `--format text` is equivalent to not setting JSON flags.
- If both are present, they must resolve to JSON mode.
  Example invalid usage: `--json --format text` (usage error).
- Machine-facing commands `fetch` and `apply` MUST support both `--json` and `--format json`.
- `list`, `search`, and `report` also support `--json` and `--format json` for automation parity.
- `add` supports JSON mode for script-driven capture acknowledgements.

## stdout/stderr boundary

| Channel | Text mode | JSON mode |
| --- | --- | --- |
| `stdout` | primary command result (tables, lists, summaries) | exactly one JSON object (`result` or `results`, or `error` when `ok=false`) |
| `stderr` | warnings, validation/runtime errors, diagnostics | optional diagnostics only; never required for machine parsing |

Contract notes:
- In JSON mode, consumers must determine outcome from JSON `ok` plus process exit code, not by parsing prose.
- Help output (`--help`) is written to `stdout`.

## usage and exit code behavior
- `0`: command completed successfully.
- `64`: usage error (invalid flags, missing required arg, incompatible option combination).
- `65`: input data error (for example malformed `apply` payload).
- `1`: runtime/operational failure (I/O, DB open/query failure, unexpected internal failure).

Rules:
- Parsing/validation failures print brief diagnostic to `stderr` and return the documented exit code.
- In JSON mode, command-level failures still emit a JSON error envelope to `stdout`.

## JSON envelope baseline
For commands using JSON mode, response envelope follows
`docs/specs/cli-service-json-contract-guideline-v1.md`:
- required top-level keys: `schema_version`, `command`, `ok`
- success: use `result` (single) or `results` (collection)
- failure: `ok=false` with `error.code`, `error.message`, and optional `error.details`

Planned schema identifiers for v1:
- `add`: `memo-cli.add.v1`
- `list`: `memo-cli.list.v1`
- `search`: `memo-cli.search.v1`
- `report`: `memo-cli.report.v1`
- `fetch`: `memo-cli.fetch.v1`
- `apply`: `memo-cli.apply.v1`

## Command Semantics

### `add`
Capture one raw inbox record.

```text
memo-cli add <text> [--source <label>] [--at <rfc3339>] [--json|--format json]
```

Behavior:
- Persists immutable raw text as a new inbox item.
- Never mutates previously captured rows.

Text output (`stdout`):
- single-line acknowledgement including item id and created timestamp.

JSON output:
- `result` includes at least: `item_id`, `created_at`, `source`, `text`.

### `list`
List captured items in deterministic order.

```text
memo-cli list [--limit <n>] [--offset <n>] [--state <all|pending|enriched>] [--json|--format json]
```

Behavior:
- Default ordering: `created_at DESC`, then `item_id DESC` as tie-breaker.
- `--state pending` means items without active enrichment.
- `--state enriched` means items with active enrichment.

Text output (`stdout`):
- table/list with id, timestamp, state, short preview.

JSON output:
- `results[]` with stable fields for pagination and state filtering.

### `search`
Search inbox and active derived fields.

```text
memo-cli search <query> [--limit <n>] [--state <all|pending|enriched>] [--json|--format json]
```

Behavior:
- Uses SQLite FTS-backed matching for raw capture and active enrichment text.
- Ranking is deterministic for score ties (`created_at DESC`, `item_id DESC`).

Text output (`stdout`):
- ranked matches with score, id, timestamp, and preview.

JSON output:
- `results[]` includes score and matched item metadata.

### `report`
Generate period summaries from capture + enrichment data.

```text
memo-cli report <week|month> [--tz <iana-tz>] [--from <rfc3339>] [--to <rfc3339>] [--json|--format json]
```

Behavior:
- `week` and `month` are canonical report windows.
- Uses capture totals and enrichment-derived categories/tags when present.
- Works even if enrichment is absent (falls back to capture-only aggregates).

Text output (`stdout`):
- summary sections for counts, top categories/tags, and open pending items.

JSON output:
- `result` contains period metadata plus aggregate fields suitable for dashboards.

### `fetch`
Machine-facing read for agent enrichment jobs.

```text
memo-cli fetch [--limit <n>] [--cursor <opaque>] [--state <pending>] [--json|--format json]
```

Behavior:
- Returns records eligible for enrichment (default state: pending).
- Cursor-based pagination for stable batch processing.
- Does not mutate data.

Text output (`stdout`):
- human summary for manual inspection (batch size and ids).

JSON output:
- `results[]` includes immutable source fields required by enrichment workers.
- `result.next_cursor` (or equivalent) for continuation.

### `apply`
Machine-facing write-back for agent enrichment results.

```text
memo-cli apply (--input <file> | --stdin) [--dry-run] [--json|--format json]
```

Behavior:
- Accepts structured enrichment payload generated from `fetch`.
- Writes derivations as new versions; raw capture remains immutable.
- Active derivation selection follows latest accepted version per item.
- `--dry-run` validates payload and reports changes without committing writes.

Text output (`stdout`):
- apply summary: accepted, skipped, failed counts.

JSON output:
- `result` includes counts and per-item status entries.
- invalid payload returns `ok=false` and exits with input/usage error code.

## End-to-end flow: capture -> agent enrichment -> report
1. Quick capture:
   - `memo-cli add "buy 1tb ssd for mom"`
   - `memo-cli add "book pediatric dentist appointment"`
2. Agent pull (machine mode):
   - `memo-cli fetch --json --limit 50 > inbox-batch.json`
3. Agent enrichment writes payload for apply:
   - Each record includes normalized fields (for example category, priority, due hints, confidence).
4. Apply enrichment:
   - `memo-cli apply --format json --input enrichment-batch.json`
5. Human summary report:
   - `memo-cli report week`
6. Service/dashboard report:
   - `memo-cli report month --json`

Expected contract outcome:
- capture is durable and append-only;
- enrichment is versioned and replaceable without rewriting raw text;
- report reflects latest active enrichment with capture fallback.

## Non-goals for this contract version
- Cloud sync or multi-device conflict resolution.
- Embedding/vector semantic search.
- Background daemon mode.
