# api-test parity spec

## Overview
`api-test` runs a suite of API checks (REST + GraphQL) from a single manifest file and emits a machine-readable results
JSON to stdout (plus optional JUnit XML). It also provides a “summary” mode that renders a human-friendly Markdown summary
from the results JSON.

It ports the behavior of the Codex Kit scripts:
- `api-test-runner/scripts/api-test.sh` (suite runner)
- `api-test-runner/scripts/api-test-summary.sh` (summary renderer)

Primary goals:
- Deterministic, CI-friendly execution (stable exit codes, JSON results schema, per-case artifacts).
- Safety defaults to prevent accidental writes in CI (explicit allowWrite + enable-writes gate).
- Secret-safe outputs (no tokens in logs; command snippets redact auth inputs).

## Entry point
Command: `api-test <command> [args]`

Commands:
- `api-test run ...` (default)
- `api-test summary ...`

## run
Usage (parity intent):
`api-test run (--suite <name> | --suite-file <path>) [--tag <tag> ...] [--only <csv>] [--skip <csv>] [--allow-writes] [--fail-fast|--continue] [--out <path>] [--junit <path>]`

### Suite selection and resolution
Ported from `api-test.sh`:

- `--suite <name>` resolves to:
  1. `tests/api/suites/<name>.suite.json`
  2. (fallback) `setup/api/suites/<name>.suite.json`
  - Override the suites directory via `API_TEST_SUITES_DIR`.
- `--suite-file <path>` uses an explicit path (relative paths resolve from repo root).
- `--suite` and `--suite-file` are mutually exclusive.

Repo root:
- Runner must execute inside a git work tree and uses repo root to resolve relative paths.

### Filters and selection semantics
Ported from `api-test.sh`:

- `--tag <tag>` is repeatable and uses AND semantics:
  - a case must include **all** selected tags to run.
- `--only <id1,id2,...>` runs only those case IDs (others are skipped with reason `not_selected`).
- `--skip <id1,id2,...>` skips listed case IDs (reason `skipped_by_id`).
- `--fail-fast` stops after the first failed case; default is continue.

### Write safety and guardrails
Ported from `api-test.sh`:

There are two gates:
1. **Per-case declaration**: `allowWrite: true` must be set on write-capable cases.
2. **Per-run enablement**: writes must be enabled by either:
   - `--allow-writes` or `API_TEST_ALLOW_WRITES_ENABLED=true`, or
   - effective environment is `local` (case env or suite defaults).

Write-capable classification:
- REST: a case is treated as write-capable when its request method is a “write” method (anything other than `GET|HEAD|OPTIONS`).
- GraphQL: a case is treated as write-capable when:
  - its operation is detected as a `mutation`, or
  - `allowWrite: true` is explicitly set (defensive classification).

Behavior:
- If a case is write-capable but `allowWrite` is false → case fails (`write_capable_case_requires_allowWrite_true` or
  `mutation_case_requires_allowWrite_true`).
- If `allowWrite` is true but writes are not enabled → case is skipped with reason `write_cases_disabled`.

### Suite manifest schema v1
The suite file is JSON and must match `version: 1`.

High-level structure:
```json
{
  "version": 1,
  "name": "smoke",
  "defaults": {
    "env": "staging",
    "noHistory": true,
    "rest": { "configDir": "setup/rest", "url": "", "token": "" },
    "graphql": { "configDir": "setup/graphql", "url": "", "jwt": "" }
  },
  "auth": {
    "provider": "rest",
    "required": true,
    "secretEnv": "API_TEST_AUTH_JSON",
    "rest": {
      "loginRequestTemplate": "setup/rest/requests/login.request.json",
      "credentialsJq": ".profiles[.profile]",
      "tokenJq": ".. | objects | .accessToken? // empty",
      "configDir": "setup/rest",
      "url": "",
      "env": "staging"
    }
  },
  "cases": [
    {
      "id": "rest.health",
      "type": "rest",
      "tags": ["smoke"],
      "env": "",
      "noHistory": true,
      "allowWrite": false,
      "configDir": "",
      "url": "",
      "token": "",
      "request": "setup/rest/requests/health.request.json"
    },
    {
      "id": "rest.auth.login_then_me",
      "type": "rest-flow",
      "tags": ["smoke"],
      "allowWrite": true,
      "loginRequest": "setup/rest/requests/login.request.json",
      "tokenJq": ".accessToken",
      "request": "setup/rest/requests/me.request.json"
    },
    {
      "id": "graphql.countries",
      "type": "graphql",
      "tags": ["smoke"],
      "allowWrite": false,
      "allowErrors": false,
      "op": "setup/graphql/operations/countries.graphql",
      "vars": "setup/graphql/operations/countries.variables.json",
      "expect": { "jq": "(.errors? | length // 0) == 0" }
    }
  ]
}
```

Case types:
- `rest`: execute a REST request file.
- `rest-flow` / `rest_flow`: execute a login REST request, extract a token (via `tokenJq`), then execute the main request
  using `ACCESS_TOKEN`.
- `graphql`: execute an operation and optional vars via the GraphQL runner.

GraphQL case assertions:
- Default: fail when `.errors` is non-empty.
- If `allowErrors=false` and `expect.jq` is omitted: also require `.data` to be a non-null object.
  - If `allowErrors=true`: `expect.jq` is required and must be evaluated (suite file is invalid without it).

### Optional CI auth via secret JSON (`.auth`)
Ported from `api-test.sh`:

Purpose:
- Avoid committing real tokens into `tokens.local.env` / `jwts.local.env` for CI runs.
- Login once per “profile”, cache the token, then inject `ACCESS_TOKEN` per-case.

Top-level behavior:
- If `.auth` is present, it must be an object.
- `secretEnv` defaults to `API_TEST_AUTH_JSON`.
- `required` defaults to `true`:
  - if `true` and the env var is missing → hard-fail the suite (exit `1`).
  - if `false` and the env var is missing → auth is disabled and the suite continues.
- Provider selection:
  - `provider` may be `rest` or `graphql` (alias `gql`).
  - If omitted, it is inferred when only one of `.auth.rest` / `.auth.graphql` is present.

Credential extraction rules (parity-critical):
- `credentialsJq` is evaluated against the secret JSON and must yield **exactly one** object for a given profile:
  - 0 matches → `auth_credentials_missing(...)` (fail)
  - >1 matches → `auth_credentials_ambiguous(...)` (fail)
- The extracted credentials object is merged into a login template:
  - REST: merged into the login request template `.body`
  - GraphQL: merged into the login variables template object

Token extraction:
- `tokenJq` defaults to a recursive search for `.accessToken` / `.access_token` / `.token` / `.jwt` (first match).

Interaction with runner config dirs:
- When injecting `ACCESS_TOKEN`, the runner uses a generated “auth config dir” that copies endpoint presets but intentionally
  does **not** copy token/JWT files, preventing token-profile defaults from interfering with `ACCESS_TOKEN` injection.

### Per-case artifacts
Ported from `api-test.sh`:
- Output directory base: `API_TEST_OUTPUT_DIR` (default: `<repo>/out/api-test-runner`)
- Per-run directory: `<base>/<runId>/` where `runId` is UTC timestamp `YYYYMMDD-HHMMSSZ`.
- Each executed case writes:
  - `<safeId>.response.json` (stdout)
  - `<safeId>.stderr.log` (stderr)

### Cleanup steps (optional)
Ported from `api-test.sh`:
- A case may define `cleanup` as an object (single step) or array of steps.
- Each step includes `type`: `rest` or `graphql`.
- Cleanup runs only when writes are enabled (or effective env is local).
- Cleanup failures:
  - are recorded in stderr logs
  - cause a passed case to become failed (`cleanup_failed`), without breaking results JSON emission.

### Results JSON schema (v1)
Ported from `api-test.sh`:

- Runner always prints JSON results to stdout.
- Optional `--out <path>` writes the same JSON to a file.

Shape:
```json
{
  "version": 1,
  "suite": "smoke",
  "suiteFile": "tests/api/suites/smoke.suite.json",
  "runId": "20260131-000000Z",
  "startedAt": "2026-01-31T00:00:00Z",
  "finishedAt": "2026-01-31T00:00:10Z",
  "outputDir": "out/api-test-runner/20260131-000000Z",
  "summary": { "total": 3, "passed": 3, "failed": 0, "skipped": 0 },
  "cases": [
    {
      "id": "rest.health",
      "type": "rest",
      "status": "passed",
      "durationMs": 12,
      "tags": ["smoke"],
      "command": "… redacted snippet …",
      "stdoutFile": "out/api-test-runner/.../rest.health.response.json",
      "stderrFile": "out/api-test-runner/.../rest.health.stderr.log",
      "message": "optional stable reason",
      "assertions": { "defaultNoErrors": "passed" }
    }
  ]
}
```

Exit codes:
- `0`: all selected cases passed.
- `2`: one or more selected cases failed.
- `1`: invalid inputs / suite schema errors / missing files.

### JUnit output (optional)
Ported from `api-test.sh`:
- When `--junit <path>` is set, emit a JUnit XML file with testcase entries for each case:
  - `skipped` elements for skipped cases
  - `failure` elements for failed cases (including command/stdoutFile/stderrFile context when available)

## summary
Ported from `api-test-summary.sh`.

Usage (parity intent):
`api-test summary [--in <results.json>] [--out <path>] [--slow <n>] [--hide-skipped] [--max-failed <n>] [--max-skipped <n>] [--no-github-summary]`

Key behaviors:
- Consumes results JSON from a file (`--in`) or stdin.
- Emits Markdown summary to stdout by default; optionally writes to `--out`.
- In GitHub Actions, may append to `$GITHUB_STEP_SUMMARY` unless `--no-github-summary` is set.

## External dependencies (inventory + policy)
Source scripts depend on:
- `bash`, `jq`, `python3`, and `git`.
- Underlying runners: `rest.sh` and `gql.sh` (which in turn depend on curl/xh/http).

Port policy for the Rust binaries:
- `rest.sh` / `gql.sh`: Eliminate (call the Rust runner implementations directly).
- `jq`: Eliminate by default via an embedded jq-like engine for `expect.jq`, token extraction, and templating.
  - Optional fallback: allow calling external `jq` only when explicitly enabled (compat mode).
- `python3`: Eliminate (JSON/JUnit/summary generation and token parsing implemented in Rust).
- `git`: Prefer to eliminate by locating repo root via filesystem (`.git`), but preserve the “must run inside a git work tree”
  guardrail and error message intent.
