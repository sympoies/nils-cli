# API testing CLIs overview

## Overview
This workspace ports the existing Bash-based API testing scripts into three Rust binaries: `api-rest`, `api-gql`, and `api-test`.
The top priority is behavioral parity with the legacy scripts: flags, defaults, exit codes, and on-disk artifacts (history files, reports, results).
These tools are designed to work with a conventional repository layout that keeps configuration under `setup/` and suite manifests under `tests/`.

## Binaries and legacy script mapping
- `api-rest` (REST)
  - `api-rest call` (or the default mode) : parity with `rest.sh`
  - `api-rest history` : parity with `rest-history.sh`
  - `api-rest report` : parity with `rest-report.sh`
- `api-gql` (GraphQL)
  - `api-gql call` (or the default mode) : parity with `gql.sh`
  - `api-gql history` : parity with `gql-history.sh`
  - `api-gql report` : parity with `gql-report.sh`
  - `api-gql schema` : parity with `gql-schema.sh`
- `api-test` (suite runner)
  - `api-test run` : parity with `api-test.sh`
  - `api-test summary` : parity with `api-test-summary.sh`

Notes:
- `api-test` executes REST and GraphQL cases by invoking the Rust equivalents (`api-rest` and `api-gql`) rather than shelling out to the legacy scripts.
- Where the legacy scripts embed their own paths (often under `$CODEX_HOME`) in history/report command snippets, the Rust ports may emit a stable Rust invocation instead (for example `api-rest ... | jq .`), while preserving option semantics.

## Shared terminology
- Setup dir
  - A protocol-specific directory that contains configuration files (and, optionally, local-only overrides).
  - Canonical locations:
    - REST: `setup/rest`
    - GraphQL: `setup/graphql`
- Config dir
  - A CLI argument (`--config-dir <dir>`) that selects or seeds discovery of the setup dir.
  - The legacy scripts resolve the final setup dir by searching upward from a seed directory for known config files; if none are found, they fall back to conventional `setup/<tool>` locations.
- Env preset
  - A short name passed via `--env <name>` that selects a base URL from `endpoints.env` (+ optional `.local` overrides).
  - REST: looks up `REST_URL_<ENV_KEY>`; GraphQL: looks up `GQL_URL_<ENV_KEY>`.
  - `<ENV_KEY>` is derived by uppercasing and replacing non-alphanumerics with underscores (for example `local-dev` → `LOCAL_DEV`).
  - If the `--env` value looks like a URL (`http://...` or `https://...`), the legacy scripts treat it as an explicit URL (equivalent to `--url`).
- Token profile
  - A named bearer token selection used to populate `Authorization: Bearer ...`.
  - REST: `--token <name>` / `REST_TOKEN_NAME` selects `REST_TOKEN_<NAME>` from `tokens.env` (+ optional `tokens.local.env`).
    - If no token profile is selected, the legacy script falls back to `ACCESS_TOKEN` (or `SERVICE_TOKEN`).
  - GraphQL: `--jwt <name>` / `GQL_JWT_NAME` selects `GQL_JWT_<NAME>` from `jwts.env` (+ optional `jwts.local.env`).
    - If no JWT profile is selected, the legacy script falls back to `ACCESS_TOKEN`.
    - If a JWT profile is selected but missing, the legacy script may auto-login by running `login.graphql` under the GraphQL setup dir to fetch a token.
- History
  - An append-only log of past invocations stored under the setup dir:
    - REST: `<setup_dir>/.rest_history`
    - GraphQL: `<setup_dir>/.gql_history`
  - A history entry is a blank-line-separated record containing a metadata header line plus a copy/paste-friendly command snippet.
  - History is enabled by default, can be disabled, and supports rotation/size limits.
- Report
  - A Markdown artifact (typically written under `<repo>/docs/`) that captures a single “case”:
    - command snippet (optional)
    - request/operation inputs
    - response output (and optional assertions / stderr)
  - Reports redact common secret fields by default and can be configured to omit or include sensitive details (for example URLs in the command snippet).

## Canonical repo layouts
The Rust CLIs aim to support the same “canonical” layouts the scripts assume, so that repositories can keep setup/config and suites in predictable places.

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
    api-test-runner/             # suite runner default output dir
```

### Layout B: Suites under `setup/` (supported fallback)
`api-test` (and the legacy `api-test.sh`) resolves `--suite <name>` to:
- `<repo>/tests/api/suites/<name>.suite.json` (preferred)
- `<repo>/setup/api/suites/<name>.suite.json` (fallback)

## Parity-critical vs best-effort
- Parity-critical
  - Flags, environment variables, and defaults (including config discovery and preset resolution)
  - Exit code semantics and error messaging intent (user-actionable guardrails)
  - Artifact behavior: history append/rotation, report generation, suite results JSON (+ optional JUnit)
  - Secret-handling defaults (redaction/masking behavior)
- Best-effort
  - Which HTTP client implementation is used internally (the scripts use `curl`/`xh`/`http`, while Rust uses its own HTTP stack)
  - Performance and portability improvements that do not change user-visible behavior or artifacts
# API testing CLIs usage

This document shows how to use the Rust ports of the Codex Kit API testing scripts:
`api-rest`, `api-gql`, and `api-test`.

See also:
- `crates/api-rest/README.md`
- `crates/api-gql/README.md`
- `crates/api-test/README.md`

## Migration mapping (legacy → Rust)

| Legacy script | Rust binary |
| --- | --- |
| `rest.sh` | `api-rest call` |
| `rest-history.sh` | `api-rest history` |
| `rest-report.sh` | `api-rest report` |
| `gql.sh` | `api-gql call` |
| `gql-history.sh` | `api-gql history` |
| `gql-report.sh` | `api-gql report` |
| `gql-schema.sh` | `api-gql schema` |
| `api-test.sh` | `api-test run` |
| `api-test-summary.sh` | `api-test summary` |

## Recommended repo layout

```text
<repo>/
  setup/
    rest/
      endpoints.env
      endpoints.local.env   # optional (local override)
      tokens.env
      tokens.local.env      # optional (local tokens; do not commit)
    graphql/
      endpoints.env
      endpoints.local.env   # optional (local override)
      jwts.env
      jwts.local.env        # optional (local jwts; do not commit)
      operations/
        login.graphql       # optional (for auto-login)
      schema.graphql        # or: schema.gql / schema.graphqls / api.graphql / api.gql
  tests/
    api/
      suites/
        smoke.suite.json
  out/
    api-test-runner/
```

## `api-rest` examples

Run a request (prints the HTTP response body JSON to stdout):

```bash
api-rest call --env staging setup/rest/requests/health.request.json
```

Override base URL directly:

```bash
api-rest call --url http://localhost:6700 setup/rest/requests/health.request.json
```

Use a token profile (selected from `setup/rest/tokens(.local).env`):

```bash
api-rest call --env staging --token service setup/rest/requests/me.request.json
```

Write a Markdown report:

```bash
api-rest report --case health --request setup/rest/requests/health.request.json --run
```

Generate a report from a saved `call` snippet (e.g. from history):

```bash
api-rest history --command-only | api-rest report-from-cmd --stdin
```

Show the rewritten `report` command (no network):

```bash
api-rest report-from-cmd --dry-run "api-rest call --env staging setup/rest/requests/health.request.json"
```

Offline mode (use a saved response body):

```bash
api-rest report-from-cmd --response out/health.response.json "api-rest call --env staging setup/rest/requests/health.request.json"
```

If you use `--response -`, stdin is reserved for the response body (the snippet must be positional):

```bash
api-rest report-from-cmd --response - "api-rest call --env staging setup/rest/requests/health.request.json" < out/health.response.json
```

Show history (default: last entry):

```bash
api-rest history
```

## `api-gql` examples

Run an operation (prints the GraphQL response body JSON to stdout):

```bash
api-gql call --env staging setup/graphql/operations/health.graphql
```

Run with variables:

```bash
api-gql call --env staging setup/graphql/operations/countries.graphql setup/graphql/operations/countries.variables.json
```

Use a JWT profile (selected from `setup/graphql/jwts(.local).env`):

```bash
api-gql call --env staging --jwt service setup/graphql/operations/me.graphql
```

Write a Markdown report:

```bash
api-gql report --case health --op setup/graphql/operations/health.graphql --run
```

Generate a report from a saved `call` snippet (e.g. from history):

```bash
api-gql history --command-only | api-gql report-from-cmd --stdin
```

Show the rewritten `report` command (no network):

```bash
api-gql report-from-cmd --dry-run "api-gql call --env staging setup/graphql/operations/health.graphql"
```

Offline mode (use a saved response body):

```bash
api-gql report-from-cmd --response out/health.response.json "api-gql call --env staging setup/graphql/operations/health.graphql"
```

If you use `--response -`, stdin is reserved for the response body (the snippet must be positional):

```bash
api-gql report-from-cmd --response - "api-gql call --env staging setup/graphql/operations/health.graphql" < out/health.response.json
```

Resolve and print the schema file:

```bash
api-gql schema --cat
```

## `api-test` examples

Run a suite (always emits results JSON to stdout):

```bash
api-test run --suite smoke
```

Write results JSON and optional JUnit XML:

```bash
api-test run --suite smoke --out out/api-test-runner/results.json --junit out/api-test-runner/junit.xml
```

Filter by tags (repeatable; AND semantics):

```bash
api-test run --suite smoke --tag smoke --tag graphql
```

Run only / skip case IDs:

```bash
api-test run --suite smoke --only rest.health,graphql.health
api-test run --suite smoke --skip graphql.health
```

Write-capable cases:

- A write-capable case must set `allowWrite: true` in the suite file.
- Writes are disabled by default (even with `allowWrite: true`).
- Enable writes with `--allow-writes` (or `API_TEST_ALLOW_WRITES_ENABLED=true`).
- Tip: keep “guardrail” negative cases (write-capable but `allowWrite: false`) in a separate suite (e.g. `guardrails.suite.json`) so your smoke suite report stays green.

```bash
api-test run --suite smoke --allow-writes
```

Render a Markdown summary from results JSON:

```bash
api-test summary --in out/api-test-runner/results.json --out out/api-test-runner/summary.md
```

CI tip (GitHub Actions):

- `api-test summary` appends to `$GITHUB_STEP_SUMMARY` by default when the env var is set.
- Use `--no-github-summary` to disable that behavior.

## Environment variables (suite runner)

Common `api-test` env vars:

- `API_TEST_OUTPUT_DIR`: base output directory (default: `<repo>/out/api-test-runner`)
- `API_TEST_ALLOW_WRITES_ENABLED=true|false`: enable write-capable cases
- `API_TEST_REST_URL`: override REST base URL for all REST/rest-flow cases
- `API_TEST_GQL_URL`: override GraphQL endpoint URL for all GraphQL cases
