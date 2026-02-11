# CLI Service JSON Contract Guideline v1

## Purpose
This document defines the required JSON contract rules for CLI commands consumed by services.

Goals:
- keep machine parsing stable;
- preserve human-readable mode as default UX;
- avoid sensitive data leakage in JSON output.

## Scope
Apply this guideline to any CLI command that is called by frontend/backend services, automation, or
other CLIs expecting structured output.

## Mode Rules
- Human-readable mode remains default unless the command contract says otherwise.
- JSON mode must be explicit (`--json` or `--format json`).
- JSON mode must not require parsing prose stderr to determine outcome.

## Required Envelope
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

## Error Contract
- `error.code` must be stable and machine-usable.
- `error.message` should be concise and human-readable.
- `error.details` is optional but recommended for structured diagnostics.
- Keep command exit code semantics documented and stable.

## Sensitive Data Policy
- Never emit local secret/token material in JSON responses.
- Do not expose fields like `access_token`, `refresh_token`, raw authorization headers, or private
  key content.
- If upstream payloads include sensitive fields, redact or drop those fields before output.

## Compatibility Rules
- Within one `schema_version`, additive fields are allowed.
- Renaming/removing required fields is breaking and requires a new `schema_version`.
- If breaking changes are required, keep prior version behavior available until consumers migrate.

## Testing Requirements
For every service-consumed JSON command, include contract tests that verify:
1. required envelope fields (`schema_version`, `command`, `ok`);
2. expected payload key presence (`result` or `results`, `error` when `ok=false`);
3. stable error envelope keys (`code`, `message`, optional `details`);
4. no secret leakage in success/failure paths.

## Recommended Documentation Pattern
- Put command-specific JSON examples in crate-level README or command docs.
- Keep this document as the generic contract baseline.
- If a crate needs stronger constraints, add a crate-local spec that references this guideline.
