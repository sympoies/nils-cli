# api-test

## Overview
`api-test` runs REST + GraphQL + gRPC + WebSocket suites from a JSON manifest, emits results JSON to stdout, and can render a Markdown summary.

## Usage
```text
Usage:
  api-test <command> [args]

Commands:
  run       Run a suite (default)
  summary   Render a Markdown summary from results JSON
```

## Commands
- `run` (default): execute a suite and write results JSON to stdout.
  Options: `--suite <name>` | `--suite-file <path>`, `--tag <tag>`, `--only <csv>`, `--skip <csv>`,
  `--allow-writes`, `--fail-fast`/`--continue`, `--out <path>`, `--junit <path>`.
- `summary`: render a Markdown summary from results JSON (stdin or `--in`).

## WebSocket suite contract (`type: websocket`)
Additive defaults:
- `defaults.websocket.configDir` (default: `setup/websocket`)
- `defaults.websocket.url` (endpoint preset or literal URL)
- `defaults.websocket.token` (token profile name)

Additive case fields:
- `type: "websocket"` (alias: `"ws"`)
- `request` (required, path to `*.ws.json` or `*.websocket.json`)
- optional: `env`, `url`, `token`, `configDir`, `noHistory`

Validation rules:
- `type: "websocket"` without `request` is rejected.
- Existing REST/GraphQL/gRPC schema contracts remain unchanged.

Runtime override:
- `API_TEST_WS_URL` overrides resolved WebSocket target for suite runs.

## gRPC suite contract (`type: grpc`)
Additive defaults:
- `defaults.grpc.configDir` (default: `setup/grpc`)
- `defaults.grpc.url` (endpoint preset or literal target)
- `defaults.grpc.token` (token profile name)

Additive case fields:
- `type: "grpc"`
- `request` (required, path to `*.grpc.json`)
- optional: `env`, `url`, `token`, `configDir`, `noHistory`

Runtime override:
- `API_TEST_GRPC_URL` overrides resolved gRPC target for suite runs.

## Mixed protocol suite example
```json
{
  "version": 1,
  "defaults": {
    "rest": {"configDir": "setup/rest"},
    "graphql": {"configDir": "setup/graphql"},
    "grpc": {"configDir": "setup/grpc", "url": "local", "token": "default"},
    "websocket": {"configDir": "setup/websocket", "url": "local", "token": "default"}
  },
  "cases": [
    {"id": "rest-health", "type": "rest", "request": "setup/rest/requests/health.request.json"},
    {"id": "gql-health", "type": "graphql", "op": "setup/graphql/operations/health.graphql"},
    {"id": "grpc-health", "type": "grpc", "request": "setup/grpc/requests/health.grpc.json"},
    {"id": "ws-health", "type": "websocket", "request": "setup/websocket/requests/health.ws.json"}
  ]
}
```

## Reuse matrix (unchanged vs additive protocol paths)
| Capability | Status | Evidence |
| --- | --- | --- |
| suite selection/filtering | unchanged | `cargo test -p nils-api-testing-core --test suite_rest_graphql_matrix` |
| run/result envelope | unchanged | `cargo test -p nils-api-testing-core --test suite_runner_loopback` |
| summary/JUnit generation | unchanged | `cargo test -p nils-api-testing-core suite::summary suite::junit` |
| grpc protocol dispatch | additive grpc | `cargo test -p nils-api-testing-core --test suite_runner_grpc_matrix` |
| websocket protocol dispatch | additive websocket | `cargo test -p nils-api-testing-core --test suite_runner_websocket_matrix` |
| suite schema validation | additive | `cargo test -p nils-api-test suite_schema` |
| env override wiring | additive | `cargo test -p nils-api-testing-core suite::runtime_tests` |

## Docs
- [Docs index](docs/README.md)
- [WebSocket adoption runbook](docs/runbooks/api-test-websocket-adoption.md)
