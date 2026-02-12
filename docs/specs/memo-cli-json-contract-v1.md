# memo-cli JSON Contract v1

## Purpose
This document defines machine-consumable JSON contracts for `memo-cli` commands:
`add`, `list`, `search`, `report`, `fetch`, and `apply`.

It is a command-specific extension of:
- `docs/specs/cli-service-json-contract-guideline-v1.md`

Human-readable mode remains the default UX. JSON mode is explicit (`--json` or `--format json`).

## Schema Versions and Stable Command Identifiers

| Command surface | Canonical `command` | `schema_version` | Success payload key |
| --- | --- | --- | --- |
| `add` | `memo-cli add` | `memo-cli.add.v1` | `result` |
| `list` | `memo-cli list` | `memo-cli.list.v1` | `results` |
| `search` | `memo-cli search` | `memo-cli.search.v1` | `results` |
| `report` | `memo-cli report` | `memo-cli.report.v1` | `result` |
| `fetch` | `memo-cli fetch` | `memo-cli.fetch.v1` | `results` |
| `apply` | `memo-cli apply` | `memo-cli.apply.v1` | `result` |

The `command` value must be stable across v1 and match the table exactly.

## Required Envelope Rules

All JSON responses must include top-level:
- `schema_version` (string)
- `command` (string)
- `ok` (boolean)

Success response:
- `ok=true`
- exactly one of:
  - `result` for a single logical object
  - `results` for collection/list outputs
- optional additive metadata (`pagination`, `meta`) is allowed.

Failure response:
- `ok=false`
- `error` object with:
  - `code` (stable machine-facing identifier)
  - `message` (concise human-readable summary)
  - optional `details` (structured diagnostics)
- `result` and `results` must not appear when `ok=false`.

## Stable Payload Contracts by Command

### `add` (`result`)
- `item_id`: stable unique item identifier (string)
- `created_at`: RFC3339 timestamp (UTC, string)
- `source`: capture source label (string)
- `text`: immutable raw memo text (string)

### `list` (`results`)
- `results[]` item fields:
  - `item_id`, `created_at`
  - `state` (`pending` or `enriched`)
  - `text_preview` (truncated safe preview)
- optional `pagination`:
  - `limit`, `offset`, `returned`

### `search` (`results`)
- `results[]` item fields:
  - `item_id`, `created_at`
  - `score` (number)
  - `matched_fields` (array of strings; example: `raw_text`, `category`)
  - `preview` (string)
- optional `meta`:
  - `query`, `limit`, `state`

### `report` (`result`)
- `result.period`: `week` or `month`
- `result.range`: object with `from`, `to`, `timezone`
- `result.totals`: object with `captured`, `enriched`, `pending`
- optional aggregate sections:
  - `top_categories[]`
  - `top_tags[]`

### `fetch` (`results`, machine-facing)
- `results[]` item fields:
  - `item_id`, `created_at`, `source`, `text`
  - `state` (expected `pending` in v1 fetch flows)
- optional `pagination`:
  - `limit`, `returned`, `next_cursor` (nullable string), `has_more` (boolean)

### `apply` (`result`, machine-facing)
- `result` summary fields:
  - `dry_run` (boolean)
  - `processed`, `accepted`, `skipped`, `failed` (non-negative integers)
  - `items[]` per-item outcomes
- `result.items[]` fields:
  - `item_id`
  - `status` (`accepted` | `skipped` | `failed`)
  - optional `derivation_version` when accepted
  - optional per-item `error` with `code`, `message`, optional `details`

## Stable Error Contract

Error envelope (all commands):
- `error.code`: stable, machine-usable code
- `error.message`: concise human-readable summary
- `error.details`: optional structured context for diagnostics/automation

Recommended stable error codes in v1:

| `error.code` | Meaning | Typical commands |
| --- | --- | --- |
| `invalid-arguments` | CLI flags/arguments are invalid or incompatible | all |
| `db-open-failed` | SQLite database open/bootstrap failed | all |
| `db-query-failed` | read query failed | `list`, `search`, `report`, `fetch` |
| `db-write-failed` | write transaction failed | `add`, `apply` |
| `invalid-cursor` | `fetch` cursor is malformed/expired for current DB state | `fetch` |
| `invalid-apply-payload` | input JSON payload schema/semantics are invalid | `apply` |
| `apply-item-conflict` | apply conflict for one or more item derivations | `apply` |
| `io-read-failed` | failed to read `--input` file or `--stdin` payload | `apply` |
| `internal-error` | unexpected failure not covered by above codes | all |

## Sensitive Data and No-Secret Policy

JSON output must never leak sensitive fields.

Rules:
- Never emit token/secret material such as `access_token`, `refresh_token`, API keys,
  authorization headers, or private key content.
- If upstream payloads include sensitive fields, drop or redact before output.
- `error.details` must follow the same policy (no raw secret/token text).

Redaction example (conceptual):
- allowed: `"details": {"redact_applied": true, "fields": ["authorization"]}`
- disallowed: raw bearer token or secret string in any field.

## Compatibility Rules

- Additive fields are allowed within each v1 schema.
- Renaming/removing stable required fields is breaking and requires a new `schema_version`.
- Consumers should parse only documented stable fields and ignore unknown additive fields.

## Examples

### `add` success (`result`)
```json
{
  "schema_version": "memo-cli.add.v1",
  "command": "memo-cli add",
  "ok": true,
  "result": {
    "item_id": "itm_20260212_0001",
    "created_at": "2026-02-12T08:15:41Z",
    "source": "cli",
    "text": "buy 1tb ssd for mom"
  }
}
```

### `list` success (`results`)
```json
{
  "schema_version": "memo-cli.list.v1",
  "command": "memo-cli list",
  "ok": true,
  "results": [
    {
      "item_id": "itm_20260212_0002",
      "created_at": "2026-02-12T08:20:11Z",
      "state": "pending",
      "text_preview": "book pediatric dentist appointment"
    },
    {
      "item_id": "itm_20260212_0001",
      "created_at": "2026-02-12T08:15:41Z",
      "state": "enriched",
      "text_preview": "buy 1tb ssd for mom"
    }
  ],
  "pagination": {
    "limit": 20,
    "offset": 0,
    "returned": 2
  }
}
```

### `search` success (`results`)
```json
{
  "schema_version": "memo-cli.search.v1",
  "command": "memo-cli search",
  "ok": true,
  "results": [
    {
      "item_id": "itm_20260212_0001",
      "created_at": "2026-02-12T08:15:41Z",
      "score": 0.992,
      "matched_fields": [
        "raw_text",
        "category"
      ],
      "preview": "buy 1tb ssd for mom"
    }
  ],
  "meta": {
    "query": "ssd",
    "limit": 20,
    "state": "all"
  }
}
```

### `report` success (`result`)
```json
{
  "schema_version": "memo-cli.report.v1",
  "command": "memo-cli report",
  "ok": true,
  "result": {
    "period": "week",
    "range": {
      "from": "2026-02-09T00:00:00Z",
      "to": "2026-02-16T00:00:00Z",
      "timezone": "UTC"
    },
    "totals": {
      "captured": 14,
      "enriched": 9,
      "pending": 5
    },
    "top_categories": [
      {
        "name": "shopping",
        "count": 4
      },
      {
        "name": "health",
        "count": 3
      }
    ]
  }
}
```

### `fetch` success (`results`, machine-facing)
```json
{
  "schema_version": "memo-cli.fetch.v1",
  "command": "memo-cli fetch",
  "ok": true,
  "results": [
    {
      "item_id": "itm_20260212_0002",
      "created_at": "2026-02-12T08:20:11Z",
      "source": "cli",
      "text": "book pediatric dentist appointment",
      "state": "pending"
    },
    {
      "item_id": "itm_20260212_0003",
      "created_at": "2026-02-12T08:25:44Z",
      "source": "mobile",
      "text": "renew passport in april",
      "state": "pending"
    }
  ],
  "pagination": {
    "limit": 2,
    "returned": 2,
    "next_cursor": "c_eyJpZCI6Iml0bV8yMDI2MDIxMl8wMDAzIn0",
    "has_more": true
  }
}
```

### `fetch` failure (invalid cursor)
```json
{
  "schema_version": "memo-cli.fetch.v1",
  "command": "memo-cli fetch",
  "ok": false,
  "error": {
    "code": "invalid-cursor",
    "message": "cursor is invalid for current database state",
    "details": {
      "cursor": "c_bad_value"
    }
  }
}
```

### `apply` success (`result`, machine-facing)
```json
{
  "schema_version": "memo-cli.apply.v1",
  "command": "memo-cli apply",
  "ok": true,
  "result": {
    "dry_run": false,
    "processed": 2,
    "accepted": 1,
    "skipped": 1,
    "failed": 0,
    "items": [
      {
        "item_id": "itm_20260212_0002",
        "status": "accepted",
        "derivation_version": 3
      },
      {
        "item_id": "itm_20260212_0003",
        "status": "skipped",
        "error": {
          "code": "apply-item-conflict",
          "message": "incoming version is older than active derivation",
          "details": {
            "active_version": 5,
            "incoming_version": 4
          }
        }
      }
    ]
  }
}
```

### `apply` failure (invalid payload)
```json
{
  "schema_version": "memo-cli.apply.v1",
  "command": "memo-cli apply",
  "ok": false,
  "error": {
    "code": "invalid-apply-payload",
    "message": "payload.items[0].item_id is required",
    "details": {
      "path": "payload.items[0].item_id"
    }
  }
}
```

### `report` failure (invalid arguments)
```json
{
  "schema_version": "memo-cli.report.v1",
  "command": "memo-cli report",
  "ok": false,
  "error": {
    "code": "invalid-arguments",
    "message": "period must be week or month",
    "details": {
      "received_period": "quarter"
    }
  }
}
```
