# api-grpc

## Overview
api-grpc executes JSON-defined GRPC request files, prints response bodies to stdout, keeps
optional history, and can generate Markdown reports.

## Usage
```text
Usage:
  api-grpc <command> [args]

Commands:
  call             Execute a request file and print the response body to stdout (default)
  history          Print the last (or last N) history entries
  report           Generate a Markdown API test report
  report-from-cmd  Generate a report from a saved `call` snippet

Help:
  api-grpc --help
  api-grpc call --help
  api-grpc history --help
  api-grpc report --help
  api-grpc report-from-cmd --help
```

## Commands
- `call` (default): Execute a request file and print the response body.
  Options: `--env <name>`, `--url <url>`, `--token <name>`, `--config-dir <dir>`, `--no-history`.
- `history`: Print the last entry or tail N entries.
  Options: `--config-dir <dir>`, `--file <path>`, `--last`, `--tail <n>`, `--command-only`.
- `report`: Generate a Markdown report for a request.
  Options: `--case <name>`, `--request <file>`, `--run` | `--response <file|->`, `--out <path>`,
  `--env <name>`, `--url <url>`, `--token <name>`, `--no-redact`, `--no-command`,
  `--no-command-url`, `--project-root <path>`, `--config-dir <dir>`.
- `report-from-cmd`: Generate a report from a saved `call` command snippet.
  Options: `--case <name>`, `--out <path>`, `--response <file|->`, `--allow-empty`, `--dry-run`,
  `--stdin`.

## Quickstart

### 1) Setup files
```text
setup/grpc/
  endpoints.env
  tokens.env
  requests/
    health.grpc.json
```

`setup/grpc/endpoints.env`
```bash
GRPC_URL_LOCAL=127.0.0.1:50051
```

`setup/grpc/tokens.env`
```bash
GRPC_TOKEN_DEFAULT=<jwt-or-access-token>
```

`setup/grpc/requests/health.grpc.json`
```json
{
  "method": "health.HealthService/Check",
  "body": {
    "service": "payments"
  },
  "plaintext": true,
  "expect": {
    "status": 0,
    "jq": ".ok == true"
  }
}
```

### 2) Call + history
```bash
api-grpc call --env local --token default setup/grpc/requests/health.grpc.json
api-grpc history --tail 5
```

### 3) Report
```bash
api-grpc report --case grpc-health --request setup/grpc/requests/health.grpc.json --run
api-grpc history --command-only | api-grpc report-from-cmd --stdin --dry-run
```

## Runtime dependency
- Unary execution uses `grpcurl` backend (`GRPCURL_BIN` can override executable path).
- Install:
  - macOS: `brew install grpcurl`

## Docs

- [Docs index](docs/README.md)
