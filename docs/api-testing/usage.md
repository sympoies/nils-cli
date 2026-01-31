# API testing CLIs usage

This document shows how to use the Rust ports of the Codex Kit API testing scripts:
`api-rest`, `api-gql`, and `api-test`.

See also:
- `docs/api-testing/overview.md`
- `docs/api-rest/spec.md`
- `docs/api-gql/spec.md`
- `docs/api-test/spec.md`

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
