# api-gql

## Overview

api-gql executes GraphQL operations (and optional variables), prints response JSON to stdout,
keeps optional history, and can generate Markdown reports.

## Usage

```text
Usage:
  api-gql <command> [args]

Commands:
  call             Execute an operation and print response JSON (default)
  history          Print the last (or last N) history entries
  report           Generate a Markdown API test report
  report-from-cmd  Generate a report from a command snippet (arg or stdin)
  schema           Resolve a schema file path (or print schema contents)

Help:
  api-gql --help
  api-gql call --help
  api-gql history --help
  api-gql report --help
  api-gql report-from-cmd --help
  api-gql schema --help
```

## Commands

- `call` (default): Execute an operation (and optional variables) and print response JSON.
  Options: `--env <name>`, `--url <url>`, `--jwt <name>`, `--config-dir <dir>`, `--list-envs`,
  `--list-jwts`, `--no-history`.
- `history`: Print the last entry or tail N entries.
  Options: `--config-dir <dir>`, `--file <path>`, `--last`, `--tail <n>`, `--command-only`.
- `report`: Generate a Markdown report for an operation.
  Options: `--case <name>`, `--op <file>`, `--vars <file>`, `--run` | `--response <file|->`,
  `--out <path>`, `--env <name>`, `--url <url>`, `--jwt <name>`, `--allow-empty`, `--no-redact`,
  `--no-command`, `--no-command-url`, `--project-root <path>`, `--config-dir <dir>`.
- `report-from-cmd`: Generate a report from a command snippet.
  Options: `--case <name>`, `--out <path>`, `--response <file|->`, `--allow-empty`, `--dry-run`,
  `--stdin`.
- `schema`: Resolve a schema file path (or print schema contents).
  Options: `--config-dir <dir>`, `--file <path>`, `--cat`.

## Auth selection

- `--jwt <name>` or `GQL_JWT_NAME` selects `GQL_JWT_<NAME>` from the setup `jwts.env`/`.local` files.
- If no JWT profile is selected, fallback uses `ACCESS_TOKEN` then `SERVICE_TOKEN`.
- History entries record the env source as `token=ACCESS_TOKEN` or `token=SERVICE_TOKEN`.

## Docs

- [Docs index](docs/README.md)
