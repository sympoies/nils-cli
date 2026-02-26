# WebSocket CLI Contract v1

## Commands

`api-websocket` supports:

- `call`
- `history`
- `report`
- `report-from-cmd`

Default command behavior:

- bare positional request path is treated as `call`.

## Exit codes

- `0`: success
- `1`: operational/validation failure
- `3`: history file exists but contains no records (`history`)

## Stdout/stderr behavior

- `call` (text mode): stdout prints the last received message.
- `history` (text mode): stdout prints selected history records.
- `report`/`report-from-cmd`: stdout prints generated report path.
- stderr is used for human-readable diagnostics in text mode.

## JSON mode

- Explicit only: `--format json`
- Supported commands: `call`, `history`
- Human-readable mode remains default.

## JSON envelope

Guideline reference:

- `docs/specs/cli-service-json-contract-guideline-v1.md`

### `call` success

```json
{
  "schema_version": "cli.api-websocket.call.v1",
  "command": "api-websocket call",
  "ok": true,
  "result": {
    "target": "ws://127.0.0.1:9001/ws",
    "last_received": "{\"ok\":true}",
    "transcript": []
  }
}
```

### `call` failure

```json
{
  "schema_version": "cli.api-websocket.call.v1",
  "command": "api-websocket call",
  "ok": false,
  "error": {
    "code": "request_not_found",
    "message": "Request file not found: ..."
  }
}
```

### `history` success

```json
{
  "schema_version": "cli.api-websocket.history.v1",
  "command": "api-websocket history",
  "ok": true,
  "result": {
    "history_file": ".../.ws_history",
    "count": 1,
    "records": ["..."]
  }
}
```

### `history` failure

```json
{
  "schema_version": "cli.api-websocket.history.v1",
  "command": "api-websocket history",
  "ok": false,
  "error": {
    "code": "history_not_found",
    "message": "History file not found: ..."
  }
}
```

## Stable error codes

### `call`

- `request_not_found`
- `request_parse_error`
- `setup_resolve_error`
- `endpoint_resolve_error`
- `auth_resolve_error`
- `jwt_validation_error`
- `websocket_execute_error`
- `expectation_failed`

### `history`

- `history_resolve_error`
- `history_not_found`
- `history_read_error`
- `history_empty`

## Secret handling

- JSON output must not include bearer token material.
- Tokens are never emitted in `result` payloads.
- history command snippets mask token values (`REDACTED`) in suite artifacts.
