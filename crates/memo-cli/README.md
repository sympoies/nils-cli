# memo-cli

## Overview
`memo-cli` is a capture-first memo CLI with an agent enrichment loop.
Default output is human-readable text. JSON is explicit and intended for service/agent callers.

## Usage
```text
Usage:
  memo-cli [--db <path>] [--format <text|json> | --json] <command> [options]

Commands:
  add <text> [--at <rfc3339>]                Capture a raw memo entry
  list [--limit <n>] [--offset <n>]         List entries (default: newest first)
  search <query> [--limit <n>]              Search raw + active derived text
  report <week|month> [--tz <iana-tz>]      Build period summaries
         [--from <rfc3339>] [--to <rfc3339>]
  fetch [--limit <n>] [--cursor <opaque>]   Pull records for enrichment workers
  apply (--input <file> | --stdin)          Apply enrichment payloads
```

## Commands
- `add`: append one immutable raw capture record.
- `add --at`: optional explicit capture time (RFC3339). Without `--at`, system time is used.
- `list`: show records with deterministic ordering and optional state filters.
- `search`: run keyword/prefix search across capture and active enrichment.
- `report`: render weekly/monthly summaries with capture fallback when enrichment is missing.
- `report --from/--to`: optional explicit range (RFC3339). Use both together.
- `fetch`: machine-facing pull for pending enrichment work.
- `apply`: machine-facing write-back for normalized enrichment results.

## JSON
- Text mode is the default for all commands.
- `--json` is shorthand for `--format json`.
- `fetch` and `apply` are machine-facing commands and support both `--json` and `--format json`.
- `list`, `search`, and `report` also support JSON mode for automation and dashboards.
- `list`/`search`/`fetch` may include additive metadata fields:
  `content_type`, `validation_status`.
- `report` may include additive metadata aggregates:
  `top_content_types`, `validation_status_totals`.
- In JSON mode, parse `stdout` only. `stderr` is diagnostic-only and not part of the data contract.
- Exit code policy: `0` success, `64` usage error, `65` input data error, `1` runtime failure.

## Examples
### Capture quickly
```bash
memo-cli add "buy 1tb ssd for mom"
memo-cli add "book pediatric dentist appointment"
memo-cli add --at 2026-02-12T10:00:00+08:00 "backfilled note"
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
      "content_type": "text",
      "validation_status": "valid",
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
memo-cli report week --tz Asia/Taipei
memo-cli report month --from 2026-02-01T00:00:00Z --to 2026-02-29T23:59:59Z --json
```

## Runbooks
- Agent workflow: `docs/runbooks/memo-cli-agent-workflow.md`
- Rollout/rollback: `docs/runbooks/memo-cli-rollout.md`

## Specs
- Command contract: `crates/memo-cli/docs/specs/memo-cli-command-contract-v1.md`
- JSON contract: `crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md`
- Storage schema: `crates/memo-cli/docs/specs/memo-cli-storage-schema-v1.md`
- Release policy: `crates/memo-cli/docs/specs/memo-cli-release-policy.md`
