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
