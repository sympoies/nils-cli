# api-test

## Overview
api-test runs REST + GraphQL + gRPC (unary) suites from a JSON manifest, emits results JSON to stdout, and can
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

## gRPC suite contract (`type: grpc`)
- Additive defaults:
  - `defaults.grpc.configDir` (default: `setup/grpc`)
  - `defaults.grpc.url` (endpoint preset or literal target)
  - `defaults.grpc.token` (token profile name)
- Additive case fields:
  - `type: "grpc"`
  - `request` (required, path to `*.grpc.json`)
  - optional: `env`, `url`, `token`, `configDir`, `noHistory`
- Validation rules:
  - `type: "grpc"` without `request` is rejected.
  - REST/GraphQL schema contracts remain unchanged.
- Runtime override:
  - `API_TEST_GRPC_URL` overrides resolved gRPC target for suite runs.

### Minimal gRPC case
```json
{
  "version": 1,
  "cases": [
    {
      "id": "grpc-health",
      "type": "grpc",
      "request": "setup/grpc/requests/health.grpc.json"
    }
  ]
}
```

### Mixed protocol suite example
```json
{
  "version": 1,
  "defaults": {
    "rest": {
      "configDir": "setup/rest"
    },
    "graphql": {
      "configDir": "setup/graphql"
    },
    "grpc": {
      "configDir": "setup/grpc",
      "url": "local",
      "token": "default"
    }
  },
  "cases": [
    {
      "id": "rest-health",
      "type": "rest",
      "request": "setup/rest/requests/health.request.json"
    },
    {
      "id": "gql-health",
      "type": "graphql",
      "op": "setup/graphql/operations/health.graphql"
    },
    {
      "id": "grpc-health",
      "type": "grpc",
      "request": "setup/grpc/requests/health.grpc.json"
    }
  ]
}
```

## Reuse matrix (unchanged vs additive grpc)
| Capability | Status | Evidence |
| --- | --- | --- |
| suite selection/filtering | unchanged | `cargo test -p nils-api-testing-core --test suite_rest_graphql_matrix` |
| run/result envelope | unchanged | `cargo test -p nils-api-testing-core --test suite_runner_loopback` |
| summary/JUnit generation | unchanged | `cargo test -p nils-api-testing-core suite::summary suite::junit` |
| grpc protocol dispatch | additive grpc | `cargo test -p nils-api-testing-core --test suite_runner_grpc_matrix` |
| grpc schema validation | additive grpc | `cargo test -p nils-api-test suite_schema` |
| grpc env override wiring | additive grpc | `cargo test -p nils-api-testing-core suite::runtime_tests` |

## Docs

- [Docs index](docs/README.md)
