# CLI Service JSON Contract Guideline v1

## Purpose

This document defines required JSON contract and consumer parsing rules for service-consumed CLI commands.

Goals:

- keep machine parsing stable;
- preserve human-readable mode as default UX;
- avoid sensitive data leakage in JSON output.

## Scope

Apply this guideline to any CLI command called by frontend/backend services, automation, or other CLIs expecting structured output.

## Mode rules

- Human-readable mode remains default unless the command contract says otherwise.
- JSON mode must be explicit (`--json` or `--format json`).
- JSON mode must not require parsing prose `stderr` to determine outcome.

## Required envelope

Every JSON response must include:

- `schema_version`: string (for example `cli.<command>.v1`)
- `command`: command identifier/path
- `ok`: boolean

Success payload shape:

```json
{
  "schema_version": "cli.<command>.v1",
  "command": "group subcommand",
  "ok": true,
  "result": {}
}
```

For collection/list outputs, use `results` instead of `result`:

```json
{
  "schema_version": "cli.<command>.v1",
  "command": "group subcommand",
  "ok": true,
  "results": []
}
```

Failure payload shape:

```json
{
  "schema_version": "cli.<command>.v1",
  "command": "group subcommand",
  "ok": false,
  "error": {
    "code": "stable-machine-code",
    "message": "human-readable summary",
    "details": {}
  }
}
```

## Consumer parsing baseline

1. Parse JSON from `stdout` only.
2. Validate envelope keys first: `schema_version`, `command`, `ok`.
3. Branch by envelope:
   - `ok=false`: handle top-level `error.code`, `error.message`, optional `error.details`.
   - `ok=true` + `result`: single-entity success.
   - `ok=true` + `results`: collection success; inspect per-item status fields when present.
4. Route by `schema_version` and `command` for command-specific handling.
5. Ignore unknown additive fields to stay forward-compatible within a schema version.

Minimal parser flow:

```text
read stdout JSON
assert keys: schema_version, command, ok
if ok == false: handle error.code
else if result exists: parse single payload
else if results exists: parse collection payload
else: invalid envelope
```

## Error contract

- `error.code` must be stable and machine-usable.
- `error.message` should be concise and human-readable.
- `error.details` is optional but recommended for structured diagnostics.
- Keep command exit code semantics documented and stable.

## Partial failure handling

- `ok=true` can still include partial failure for batch/collection commands.
- Consumers should parse successful items first, isolate failed targets, and retry failed targets only.
- Do not collapse partial failures into full success without surfacing unresolved targets.

## Retry and fallback guidance

| Scenario | Signal | Guidance |
| --- | --- | --- |
| Invalid usage | exit `64` and/or `error.code=invalid-arguments` | Do not retry; fix caller arguments. |
| Confirmation-required operation | `ok=false` with stable confirmation code | Ask for explicit confirmation, then rerun with explicit confirmation flag. |
| Transient command-level failure | `ok=false` with timeout/network/auth endpoint code | Retry with bounded exponential backoff. |
| Partial failure in collection mode | `ok=true` with some item `status=error` | Accept succeeded items; retry failed targets only. |
| No fresh remote data available | repeated transient failures | Fallback to service-side last-success snapshot; do not parse text mode. |

## Sensitive data policy

- Never emit local secret/token material in JSON responses.
- Do not expose fields like `access_token`, `refresh_token`, raw authorization headers, or private key content.
- If upstream payloads include sensitive fields, redact or drop those fields before output.

## Compatibility rules

- Within one `schema_version`, additive fields are allowed.
- Renaming/removing required fields is breaking and requires a new `schema_version`.
- If breaking changes are required, keep prior version behavior available until consumers migrate.

## Testing requirements

For every service-consumed JSON command, include contract tests that verify:

1. required envelope fields (`schema_version`, `command`, `ok`);
2. expected payload key presence (`result` or `results`, `error` when `ok=false`);
3. stable error envelope keys (`code`, `message`, optional `details`);
4. no secret leakage in success/failure paths.

## Documentation pattern

- Put command-specific JSON examples in crate-level docs.
- Keep this document as the shared consumer baseline.
- If a crate needs stronger constraints, add a crate-local spec that references this guideline.
