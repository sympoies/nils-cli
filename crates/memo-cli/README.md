# memo-cli

## Overview
`memo-cli` is a capture-first memo CLI with an agent enrichment loop.
Default output is human-readable text. JSON is explicit and intended for service/agent callers.

## Usage
```text
Usage:
  memo-cli [--db <path>] [--format <text|json> | --json] <command> [options]

Commands:
  add <text>                                 Capture a raw memo entry
  list [--limit <n>] [--offset <n>]         List entries (default: newest first)
  search <query> [--limit <n>]              Search raw + active derived text
  report <week|month> [--tz <iana-tz>]      Build period summaries
  fetch [--limit <n>] [--cursor <opaque>]   Pull records for enrichment workers
  apply (--input <file> | --stdin)          Apply enrichment payloads
```

## Commands
- `add`: append one immutable raw capture record.
- `list`: show records with deterministic ordering and optional state filters.
- `search`: run keyword/prefix search across capture and active enrichment.
- `report`: render weekly/monthly summaries with capture fallback when enrichment is missing.
- `fetch`: machine-facing pull for pending enrichment work.
- `apply`: machine-facing write-back for normalized enrichment results.

## JSON
- Text mode is the default for all commands.
- `--json` is shorthand for `--format json`.
- `fetch` and `apply` are machine-facing commands and support both `--json` and `--format json`.
- `list`, `search`, and `report` also support JSON mode for automation and dashboards.
- In JSON mode, parse `stdout` only. `stderr` is diagnostic-only and not part of the data contract.
- Exit code policy: `0` success, `64` usage error, `65` input data error, `1` runtime failure.

## Examples
### Capture quickly
```bash
memo-cli add "buy 1tb ssd for mom"
memo-cli add "book pediatric dentist appointment"
```

### Agent enrichment loop
```bash
memo-cli fetch --json --limit 50 > inbox-batch.json
memo-cli apply --format json --input enrichment-batch.json
```

Example `enrichment-batch.json` payload:

```json
{
  "agent_run_id": "agent-run-20260212",
  "items": [
    {
      "item_id": "itm_00000001",
      "derivation_hash": "hash-itm-00000001-v1",
      "summary": "buy ssd for mom",
      "category": "shopping",
      "normalized_text": "buy 1tb ssd for mom",
      "confidence": 0.93,
      "tags": ["family", "shopping"],
      "payload": {
        "source": "memo-agent",
        "notes": "priority this week"
      }
    }
  ]
}
```

Pagination note:

```bash
memo-cli fetch --json --limit 20 --cursor itm_00000042
```

### Human and machine reports
```bash
memo-cli report week
memo-cli report month --json
```

## Runbooks
- Agent workflow: `docs/runbooks/memo-cli-agent-workflow.md`
- Rollout/rollback: `docs/runbooks/memo-cli-rollout.md`
