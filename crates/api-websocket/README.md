# api-websocket

## Overview

`api-websocket` executes deterministic WebSocket request files, writes optional call history, and generates Markdown reports.
It follows the same CLI conventions as `api-rest`, `api-gql`, and `api-grpc`.

## Transport decision

- Selected backend: native Rust transport via `tungstenite` in `api-testing-core::websocket::runner`.
- Rejected backend: external adapter (`websocat`-style shell-out) for MVP.
- Revisit when:
  - streaming/session orchestration needs async multiplexing beyond scripted send/receive steps;
  - platform/runtime behavior diverges in CI and a swap behind the transport boundary is justified.

## Runtime dependency policy

- Runtime dependency policy: no extra external binary is required for WebSocket execution.
- `api-websocket` uses the embedded Rust transport path only.

## Setup and naming conventions

Canonical setup directory: `setup/websocket`.

### Endpoint variables

- `WS_URL_<PROFILE>` in `setup/websocket/endpoints.env` (optional local override: `endpoints.local.env`).
- `WS_ENV_DEFAULT` can set a default endpoint profile.
- `WS_URL` can force an explicit URL.

### Token variables

- `WS_TOKEN_<PROFILE>` in `setup/websocket/tokens.env` (optional local override: `tokens.local.env`).
- `WS_TOKEN_NAME` chooses a token profile.
- If no profile is selected, fallback envs are `ACCESS_TOKEN` then `SERVICE_TOKEN`.

### URL/token precedence

URL precedence (`call`):

1. `--url`
2. `--env` (profile lookup via `WS_URL_<PROFILE>`, or literal `ws://`/`wss://`)
3. `WS_URL`
4. `WS_ENV_DEFAULT` profile
5. default `ws://127.0.0.1:9001/ws`

Token precedence (`call`):

1. `--token`
2. `WS_TOKEN_NAME`
3. profile lookup via `WS_TOKEN_<PROFILE>`
4. env fallback `ACCESS_TOKEN` then `SERVICE_TOKEN`

### History

- Default history file: `<setup_dir>/.ws_history`
- Override: `WS_HISTORY_FILE`
- Controls: `WS_HISTORY_ENABLED`, `WS_HISTORY_MAX_MB`, `WS_HISTORY_ROTATE_COUNT`, `WS_HISTORY_LOG_URL_ENABLED`

## Request schema (v1)

See [`docs/specs/websocket-request-schema-v1.md`](docs/specs/websocket-request-schema-v1.md).

Quick example:

```json
{
  "url": "ws://127.0.0.1:9001/ws",
  "steps": [
    {"type": "send", "text": "ping"},
    {"type": "receive", "expect": {"jq": ".ok == true"}},
    {"type": "close"}
  ],
  "expect": {"textContains": "ok"}
}
```

## Commands

- `call` (default): execute request and print the last received message.
- `history`: print last entry or tail entries.
- `report`: generate Markdown report from `--run` or `--response`.
- `report-from-cmd`: reconstruct `report` args from a saved `call` snippet.

## JSON contract (`--format json`)

Supported for `call` and `history`.

Success envelope:

```json
{
  "schema_version": "cli.api-websocket.call.v1",
  "command": "api-websocket call",
  "ok": true,
  "result": {}
}
```

Failure envelope:

```json
{
  "schema_version": "cli.api-websocket.call.v1",
  "command": "api-websocket call",
  "ok": false,
  "error": {
    "code": "stable-machine-code",
    "message": "human-readable summary"
  }
}
```

Full CLI/JSON contract: [`docs/specs/websocket-cli-contract-v1.md`](docs/specs/websocket-cli-contract-v1.md)

## Quickstart

```bash
api-websocket call --env local setup/websocket/requests/health.ws.json
api-websocket call --format json --url ws://127.0.0.1:9001/ws setup/websocket/requests/health.ws.json
api-websocket history --tail 5
api-websocket report --case ws-health --request setup/websocket/requests/health.ws.json --run
api-websocket history --command-only | api-websocket report-from-cmd --stdin --dry-run
```

## Docs

- [Docs index](docs/README.md)
- [Request schema v1](docs/specs/websocket-request-schema-v1.md)
- [CLI contract v1](docs/specs/websocket-cli-contract-v1.md)
