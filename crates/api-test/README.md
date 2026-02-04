# api-test

## Overview
api-test runs REST + GraphQL suites from a JSON manifest, emits results JSON to stdout, and can
render a Markdown summary.

## Usage
```text
Usage:
  api-test <command> [args]

Commands:
  run       Run a suite (default)
  summary   Render a Markdown summary from results JSON

Help:
  api-test --help
  api-test run --help
  api-test summary --help
```

## Commands
- `run` (default): Execute a suite and write results JSON to stdout.
  Options: `--suite <name>` | `--suite-file <path>`, `--tag <tag>`, `--only <csv>`, `--skip <csv>`,
  `--allow-writes`, `--fail-fast`/`--continue`, `--out <path>`, `--junit <path>`.
- `summary`: Render a Markdown summary from results JSON (stdin or `--in`).
  Options: `--in <path>`, `--out <path>`, `--slow <n>`, `--hide-skipped`, `--max-failed <n>`,
  `--max-skipped <n>`, `--no-github-summary`.
