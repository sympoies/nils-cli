# API testing CLIs overview

## Overview
This workspace ships three Rust API testing CLIs (`api-rest`, `api-gql`, `api-test`) plus the shared library crate
`api-testing-core`. Behavioral parity with the legacy scripts remains the top priority: flags, defaults, exit codes,
and on-disk artifacts (history files, reports, results).

Detailed parity specs live with each binary:
- `crates/api-rest/README.md`
- `crates/api-gql/README.md`
- `crates/api-test/README.md`

This README focuses on the shared repository layout, cross-CLI concepts, and the `api-testing-core` surface area.

## Binaries and legacy script mapping
- `api-rest` (REST)
  - `api-rest call` (default) : parity with `rest.sh`
  - `api-rest history` : parity with `rest-history.sh`
  - `api-rest report` : parity with `rest-report.sh`
- `api-gql` (GraphQL)
  - `api-gql call` (default) : parity with `gql.sh`
  - `api-gql history` : parity with `gql-history.sh`
  - `api-gql report` : parity with `gql-report.sh`
  - `api-gql schema` : parity with `gql-schema.sh`
- `api-test` (suite runner)
  - `api-test run` : parity with `api-test.sh`
  - `api-test summary` : parity with `api-test-summary.sh`

Notes:
- `api-test` executes REST/GraphQL cases through shared core runners (no shelling out to scripts or the binaries).
- `api-rest`/`api-gql` also provide `report-from-cmd` as a Rust-only convenience for replaying saved `call` snippets.

## api-testing-core scope
`api-testing-core` is a library crate used by all three CLIs. Key modules:
- `config`: setup dir discovery for REST/GraphQL configs.
- `env_file`: `.env` parsing + key normalization helpers.
- `cli_*`: shared CLI helpers (endpoint resolution, history I/O, report args, CLI utilities).
- `history`, `report`, `markdown`, `redact`, `cmd_snippet`: shared report/history rendering and snippet parsing.
- `rest`: request schema, runner, expect/cleanup logic, report rendering.
- `graphql`: schema/vars loading, auth/JWT resolution, runner, expect/allow-errors, report rendering, mutation detection.
- `suite`: suite schema v1, path resolution, filters, safety gates, auth integration, runner, cleanup, results, summary, JUnit.

## Shared terminology
- Setup dir
  - Protocol-specific config directory.
  - Canonical locations: REST `setup/rest`, GraphQL `setup/graphql`.
- Config dir
  - CLI arg `--config-dir <dir>` that seeds setup-dir discovery.
  - Discovery searches upward for known config files; fallback uses the canonical `setup/<tool>` when applicable.
- Env preset
  - `--env <name>` selects a base URL from `endpoints.env` (+ optional `.local` overrides).
  - REST: `REST_URL_<ENV_KEY>`; GraphQL: `GQL_URL_<ENV_KEY>`.
  - If the value looks like `http(s)://...`, it is treated as a direct URL (like `--url`).
- Token/JWT profile
  - REST: `--token <name>` or `REST_TOKEN_NAME` selects `REST_TOKEN_<NAME>`.
  - GraphQL: `--jwt <name>` or `GQL_JWT_NAME` selects `GQL_JWT_<NAME>`.
  - REST fallback: `ACCESS_TOKEN`, then `SERVICE_TOKEN` if no profile is selected.
  - GraphQL fallback: `ACCESS_TOKEN`, then `SERVICE_TOKEN` if no profile is selected.
- History
  - REST: `<setup_dir>/.rest_history`, GraphQL: `<setup_dir>/.gql_history`.
  - Enabled by default, can be disabled, and supports rotation/size limits.
- Report
  - Markdown artifact (usually under `<repo>/docs/`) capturing request/operation, response, and optional assertions.
  - Redacts common secret fields by default, with opt-out flags for debugging.
- Suite
  - JSON manifest (`version: 1`) that drives `api-test run` and defines cases, defaults, auth, and cleanup.

## Canonical repo layouts
The CLIs support the same layouts the legacy scripts assume.

### Layout A: App repo with `setup/` + `tests/` (recommended)
```text
<repo>/
  setup/
    rest/
      endpoints.env
      endpoints.local.env        # optional (local override)
      tokens.env
      tokens.local.env           # optional (local tokens; do not commit)
    graphql/
      endpoints.env
      endpoints.local.env        # optional (local override)
      jwts.env
      jwts.local.env             # optional (local tokens; do not commit)
      schema.env                 # optional (sets GQL_SCHEMA_FILE)
      schema.local.env           # optional (local override)
      schema.graphql             # or: schema.gql / schema.graphqls / api.graphql / api.gql
      operations/                # optional (login.graphql, shared ops, etc.)
  tests/
    api/
      suites/
        <name>.suite.json
  out/
    api-test-runner/             # suite runner output base dir
```

### Layout B: Suites under `setup/` (fallback)
`api-test` resolves `--suite <name>` to:
- `<repo>/tests/api/suites/<name>.suite.json` (preferred)
- `<repo>/setup/api/suites/<name>.suite.json` (fallback)

### Layout C: Custom suites directory
- `API_TEST_SUITES_DIR=<path>` overrides the suites directory for `--suite <name>`.

## Quickstart examples

### `api-rest`
Run a request:
```bash
api-rest call --env staging setup/rest/requests/health.request.json
```

Write a report:
```bash
api-rest report --case health --request setup/rest/requests/health.request.json --run
```

Generate a report from a saved snippet:
```bash
api-rest history --command-only | api-rest report-from-cmd --stdin
```

### `api-gql`
Run an operation:
```bash
api-gql call --env staging setup/graphql/operations/health.graphql
```

Write a report:
```bash
api-gql report --case health --op setup/graphql/operations/health.graphql --run
```

Resolve and print schema:
```bash
api-gql schema --cat
```

### `api-test`
Run a suite (always emits results JSON to stdout):
```bash
api-test run --suite smoke
```

Use an explicit suite file:
```bash
api-test run --suite-file tests/api/suites/smoke.suite.json
```

Write results JSON + JUnit:
```bash
api-test run --suite smoke --out out/api-test-runner/results.json --junit out/api-test-runner/junit.xml
```

Render a Markdown summary:
```bash
api-test summary --in out/api-test-runner/results.json --out out/api-test-runner/summary.md
```

## Suite runner behavior (high-level)
- Results JSON is always emitted to stdout. `--out` writes an additional copy.
- Output directory base defaults to `<repo>/out/api-test-runner` (override via `API_TEST_OUTPUT_DIR`).
- Each run creates `<output_dir>/<run_id>/` where `run_id` is `YYYYMMDD-HHMMSSZ`.
- Per-case artifacts include `<case>.response.json` and `<case>.stderr.log`, referenced in results JSON.
- Exit code is `2` when any case fails; otherwise `0`.
- Write safety is two-step:
  - Case must set `allowWrite: true`.
  - Writes must be enabled via `--allow-writes`, `API_TEST_ALLOW_WRITES_ENABLED=true`, or `env: local`.
- Optional suite auth can derive tokens from secret JSON in `API_TEST_AUTH_JSON`.
- Optional cleanup steps run after the main case; cleanup failures mark the case failed.

## Suite runner environment variables
- `API_TEST_OUTPUT_DIR`: override the base output directory.
- `API_TEST_SUITES_DIR`: override the suites directory used by `--suite`.
- `API_TEST_ALLOW_WRITES_ENABLED`: enable write-capable cases.
- `API_TEST_REST_URL`: override REST base URL for all REST/rest-flow cases.
- `API_TEST_GQL_URL`: override GraphQL endpoint URL for all GraphQL cases.
- `API_TEST_AUTH_JSON`: credentials JSON used by suite auth (default key name).
- `GITHUB_STEP_SUMMARY`: when set, `api-test summary` appends Markdown output (disable via `--no-github-summary`).

## Docs

- [Docs index](docs/README.md)
