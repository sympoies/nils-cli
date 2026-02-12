# memo-cli Agent Workflow

## Purpose
This runbook defines a minimal capture -> fetch -> apply -> report loop for automation scripts.

## 1. Capture raw items
```bash
memo-cli add "buy 1tb ssd for mom"
memo-cli add "book pediatric dentist appointment"
```

## 2. Fetch pending items for agents
```bash
memo-cli fetch --json --limit 50 > inbox-batch.json
```

Expected JSON shape:
- top-level: `schema_version`, `command`, `ok`, `results`
- `results[]`: `item_id`, `created_at`, `source`, `text`, `state`
- optional `pagination`: `limit`, `returned`, `next_cursor`, `has_more`

When `pagination.has_more=true`, continue with:
```bash
memo-cli fetch --json --limit 50 --cursor <next_cursor>
```

## 3. Apply agent derivations
Prepare `enrichment-batch.json`:
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
      "tags": ["shopping", "family"],
      "payload": {
        "source": "memo-agent"
      }
    }
  ]
}
```

Apply:
```bash
memo-cli apply --json --input enrichment-batch.json
```

Notes:
- `derivation_hash` drives idempotency; same hash on same `item_id` becomes `skipped`.
- `--dry-run` validates and returns predicted versions without writing rows.

## 4. Validate with search and report
```bash
memo-cli search "ssd" --json
memo-cli report week
memo-cli report month --json
```

## 5. Failure handling
- Invalid payload returns `ok=false` with `error.code=invalid-apply-payload`.
- Cursor mismatch returns `ok=false` with `error.code=invalid-cursor`.
- Per-item conflicts are reported inside `result.items[].error` with `code=apply-item-conflict`.
- In text mode, warnings are sent to `stderr`; `stdout` remains primary result output.

## 6. Fallback behavior on apply validation failures
When `apply` fails validation or conflict rates spike:
1. Pause automation writes:
   - stop all `memo-cli apply` jobs.
2. Keep capture and read workflows active:
   - continue `memo-cli add`, `memo-cli list`, `memo-cli search`, `memo-cli report`.
3. Use dry-run diagnostics before re-enable:
   - `memo-cli apply --json --dry-run --stdin < enrichment-batch.json`
4. Re-enable writes only after:
   - payloads pass validation,
   - contract tests pass,
   - and rollout checks in `memo-cli-rollout.md` are green.
