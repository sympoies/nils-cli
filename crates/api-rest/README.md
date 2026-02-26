# api-rest

## Overview

api-rest executes JSON-defined REST request files, prints response bodies to stdout, keeps
optional history, and can generate Markdown reports.

## Usage

```text
Usage:
  api-rest <command> [args]

Commands:
  call             Execute a request file and print the response body to stdout (default)
  history          Print the last (or last N) history entries
  report           Generate a Markdown API test report
  report-from-cmd  Generate a report from a saved `call` snippet

Help:
  api-rest --help
  api-rest call --help
  api-rest history --help
  api-rest report --help
  api-rest report-from-cmd --help
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

## Docs

- [Docs index](docs/README.md)
