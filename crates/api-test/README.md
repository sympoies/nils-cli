# api-test parity spec

## Overview
`api-test` runs a suite of API checks (REST + GraphQL) from a single manifest file and emits a machine-readable results
JSON to stdout (plus optional JUnit XML). It also provides a "summary" mode that renders a human-friendly Markdown summary
from the results JSON.

It is the Rust port of the Codex Kit scripts:
- `api-test-runner/scripts/api-test.sh` (suite runner)
- `api-test-runner/scripts/api-test-summary.sh` (summary renderer)

Notes:
- `api-test` executes cases via the Rust runners (`api-rest` and `api-gql`) under `api-testing-core` rather than shelling
  out to the legacy scripts.
- Command snippets in results intentionally mimic the CLI surface and redact secrets.

Primary goals:
- Deterministic, CI-friendly execution (stable exit codes, JSON results schema, per-case artifacts).
- Safety defaults to prevent accidental writes in CI (explicit allowWrite + enable-writes gate).
- Secret-safe outputs (no tokens in logs; command snippets redact auth inputs).

See also: `crates/api-testing-core/README.md`.

## Entry point
Command: `api-test <command> [args]`

Commands:
- `api-test run ...` (default)
- `api-test summary ...`

## run
Usage (parity intent):
`api-test run (--suite <name> | --suite-file <path>) [--tag <tag> ...] [--only <csv>] [--skip <csv>] [--allow-writes] [--fail-fast|--continue] [--out <path>] [--junit <path>]`

### Suite selection and repo root
- Runner must execute inside a git work tree and uses repo root to resolve relative paths.
- `--suite <name>` resolves to:
  1. `tests/api/suites/<name>.suite.json`
  2. (fallback) `setup/api/suites/<name>.suite.json`
  - Override the suites directory via `API_TEST_SUITES_DIR`.
- `--suite-file <path>` uses an explicit path (relative paths resolve from repo root).
- `--suite` and `--suite-file` are mutually exclusive.

### Filters and selection semantics
- `--tag <tag>` is repeatable and uses AND semantics:
  - a case must include **all** selected tags to run.
- `--only <id1,id2,...>` runs only those case IDs (others are skipped with reason `not_selected`).
- `--skip <id1,id2,...>` skips listed case IDs (reason `skipped_by_id`).
- `--fail-fast` stops after the first failed case; default is continue.

### Write safety and guardrails
There are two gates:
1. **Per-case declaration**: `allowWrite: true` must be set on write-capable cases.
2. **Per-run enablement**: writes must be enabled by either:
   - `--allow-writes` or `API_TEST_ALLOW_WRITES_ENABLED=true`, or
   - effective environment is `local` (case env or suite defaults).

Write-capable classification:
- REST: a case is treated as write-capable when its request method is a "write" method (anything other than `GET|HEAD|OPTIONS`).
- GraphQL: a case is treated as write-capable when:
  - its operation is detected as a `mutation`, or
  - `allowWrite: true` is explicitly set (defensive classification).

Behavior:
- If a case is write-capable but `allowWrite` is false -> case fails (`write_capable_case_requires_allowWrite_true` or
  `mutation_case_requires_allowWrite_true`).
- If `allowWrite` is true but writes are not enabled -> case is skipped with reason `write_cases_disabled`.

### Execution model
- REST cases run via the Rust REST runner.
- GraphQL cases run via the Rust GraphQL runner.
- `rest-flow` runs a login request, extracts a token, then runs the main request.
- Command snippets in results use the Rust CLIs and always redact tokens.

### Suite manifest schema v1
The suite file is JSON and must match `version: 1` (camelCase fields). Defaults shown below are applied when fields are
omitted.

```json
{
  "version": 1,
  "name": "smoke",
  "defaults": {
    "env": "staging",
    "noHistory": false,
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
      "tokenJq": ".accessToken",
      "configDir": "",
      "url": "",
      "env": ""
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
      "id": "rest_flow.me",
      "type": "rest-flow",
      "tags": ["smoke"],
      "allowWrite": true,
      "loginRequest": "setup/rest/requests/login.request.json",
      "request": "setup/rest/requests/me.request.json",
      "tokenJq": ".accessToken"
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

Key notes:
- `defaults.rest.configDir` defaults to `setup/rest`; `defaults.graphql.configDir` defaults to `setup/graphql`.
- `cases[*].noHistory` overrides `defaults.noHistory`.
- `cases[*].allowWrite` defaults to `false`.
- `rest-flow` `tokenJq` defaults to:
  `.. | objects | (.accessToken? // .access_token? // .token? // empty) | select(type=="string" and length>0) | .`
- GraphQL assertions:
  - When `allowErrors=false` (default): fail when `.errors` is non-empty.
  - If `allowErrors=false` and `expect.jq` is omitted: also require `.data` to be a non-null object.
  - If `allowErrors=true`: `expect.jq` is required and must be evaluated.

### Optional CI auth via secret JSON (`.auth`)
Purpose:
- Avoid committing real tokens into `tokens.local.env` / `jwts.local.env` for CI runs.
- Login once per "profile", cache the token, then inject `ACCESS_TOKEN` per-case.

Top-level behavior:
- If `.auth` is present, it must be an object.
- `secretEnv` defaults to `API_TEST_AUTH_JSON` and must be a valid env var name.
- `required` defaults to `true`:
  - if `true` and the env var is missing -> hard-fail the suite (exit `1`).
  - if `false` and the env var is missing -> auth is disabled and the suite continues (a note is written to
    `auth.disabled.log` in the run directory).
- Provider selection:
  - `provider` may be `rest` or `graphql` (alias `gql`).
  - If omitted, it is inferred when only one of `.auth.rest` / `.auth.graphql` is present.
  - If both are present, `provider` is required.

Credential extraction rules (parity-critical):
- `credentialsJq` is evaluated against the secret JSON and must yield **exactly one** object for a given profile:
  - 0 matches -> `auth_credentials_missing(...)` (fail)
  - >1 matches -> `auth_credentials_ambiguous(...)` (fail)

REST provider:
- Required fields: `loginRequestTemplate`, `credentialsJq`.
- Credentials are merged into the login request template `.body` object.
- `tokenJq` defaults to a recursive search for `.accessToken` / `.access_token` / `.token` / `.jwt`.

GraphQL provider:
- Required fields: `loginOp`, `loginVarsTemplate`, `credentialsJq`.
- Credentials are merged into the login variables template object.
- `tokenJq` defaults to the same recursive accessToken search as REST.

### Cleanup steps (optional)
A case may define `cleanup` as an object (single step) or array of steps.

Behavior:
- Cleanup runs only when writes are enabled (or effective env is `local`).
- Cleanup failures are recorded in stderr logs and can flip a passed case to failed (`cleanup_failed`).
- Cleanup never suppresses results JSON emission.

REST cleanup step fields:
- `type`: `rest`
- `method`: default `DELETE`
- `pathTemplate`: required (must render to a `/path`)
- `vars`: optional object of string values used by `pathTemplate`
- `configDir`, `url`, `env`, `token`: optional overrides
- `expect.status` / `expectStatus`: defaults to `204` for DELETE, otherwise `200`
- `expect.jq` / `expectJq`: optional jq assertion

GraphQL cleanup step fields:
- `type`: `graphql` (alias `gql`)
- `op`: required operation file path
- `configDir`, `url`, `env`, `jwt`: optional overrides
- `allowErrors`: default `false`; if `true`, `expect.jq` is required
- `varsJq`: extract a vars object from the main response JSON
- `varsTemplate`: template file rendered against main response
- `vars`: path to a JSON file (or `null` to disable vars)

Cleanup artifacts (per step):
- `<safeId>.cleanup.<n>.request.json`
- `<safeId>.cleanup.<n>.response.json`
- `<safeId>.cleanup.<n>.stderr.log`

### Per-case artifacts
- Output directory base: `API_TEST_OUTPUT_DIR` (default: `<repo>/out/api-test-runner`).
- Per-run directory: `<base>/<runId>/` where `runId` is UTC timestamp `YYYYMMDD-HHMMSSZ`.
- Each executed case writes:
  - `<safeId>.response.json` (stdout)
  - `<safeId>.stderr.log` (stderr)

### Results JSON schema (v1)
- Runner always prints JSON results to stdout.
- Optional `--out <path>` writes the same JSON to a file (path is repo-root relative).

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
      "command": "... redacted snippet ...",
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
When `--junit <path>` is set, emit a JUnit XML file with testcase entries for each case:
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

## Environment variables (suite runner)
Common `api-test` env vars:
- `API_TEST_SUITES_DIR`: override suite lookup directory for `--suite`.
- `API_TEST_OUTPUT_DIR`: base output directory (default: `<repo>/out/api-test-runner`).
- `API_TEST_ALLOW_WRITES_ENABLED=true|false`: enable write-capable cases.
- `API_TEST_REST_URL`: override REST base URL for all REST/rest-flow cases.
- `API_TEST_GQL_URL`: override GraphQL endpoint URL for all GraphQL cases.

# api-test fixtures

These fixtures define deterministic scenarios for end-to-end tests of the suite runner and summary renderer.
Tests should use a local HTTP server that can serve both REST and GraphQL endpoints and should build temporary directory
layouts under a temp "repo root" (including a `.git` directory if the runner requires it).

## run: minimal mixed suite (REST + GraphQL) passes
- Setup:
  - Create `tests/api/suites/smoke.suite.json` with:
    - one REST case (`type=rest`) targeting a local server `GET /health` returning `200 {"ok":true}`.
    - one GraphQL case (`type=graphql`) targeting `POST /graphql` returning `{"data":{"health":{"ok":true}}}`.
  - Configure `defaults.noHistory=true` to avoid creating history files during tests.
- Command: `api-test run --suite smoke`
- Expect:
  - exit `0`
  - stdout is valid JSON with `summary.failed==0`
  - stderr contains a one-line summary including `outputDir=...`

## run: selection filters (only / skip / tag)
- Setup:
  - Suite contains 3 cases:
    - `tags=["smoke","shard:0"]`
    - `tags=["smoke","shard:1"]`
    - `tags=["slow"]`
- Commands:
  - `api-test run --suite smoke --tag smoke --tag shard:0`
  - `api-test run --suite smoke --only rest.health,graphql.health`
  - `api-test run --suite smoke --skip graphql.health`
- Expect:
  - `--tag` uses AND semantics: only shard:0 case executes; others are skipped with `tag_mismatch`.
  - `--only` skips non-selected cases with `not_selected`.
  - `--skip` skips listed cases with `skipped_by_id`.

## run: fail-fast
- Setup:
  - First case fails deterministically.
  - Second case would pass.
- Command: `api-test run --suite smoke --fail-fast`
- Expect:
  - exit `2`
  - only the first case has stdout/stderr artifacts

## run: REST write-capable without allowWrite fails
- Setup:
  - REST request file uses method `POST` (or another write method).
  - Case has `allowWrite=false`.
- Command: `api-test run --suite smoke`
- Expect:
  - case `status=failed`
  - `message=write_capable_case_requires_allowWrite_true`
  - overall exit `2`

## run: allowWrite true but writes disabled skips (non-local env)
- Setup:
  - Case has `allowWrite=true`
  - effective env is not local (for example `staging`)
  - do not set `--allow-writes` and do not set `API_TEST_ALLOW_WRITES_ENABLED=true`
- Command: `api-test run --suite smoke`
- Expect:
  - case `status=skipped`
  - `message=write_cases_disabled`
  - suite can still exit `0` if no executed cases failed

## run: GraphQL mutation requires allowWrite
- Setup:
  - GraphQL operation file contains a real `mutation` operation definition.
  - Case has `allowWrite=false`.
- Command: `api-test run --suite smoke`
- Expect:
  - case `status=failed`
  - `message=mutation_case_requires_allowWrite_true`
  - overall exit `2`

## run: GraphQL allowErrors=true requires expect.jq (schema error)
- Setup:
  - GraphQL case sets `allowErrors=true` but omits `expect.jq`.
- Command: `api-test run --suite smoke`
- Expect:
  - suite is invalid and runner exits `1` with an actionable message

## run: rest-flow login then request
- Setup:
  - `type=rest-flow` case includes:
    - `loginRequest` that returns JSON body including `{"accessToken":"<token>"}`.
    - `request` that requires `Authorization: Bearer <token>` to return 200.
    - `tokenJq` either omitted (use default) or set explicitly (e.g. `.accessToken`).
- Command: `api-test run --suite smoke`
- Expect:
  - case passes with `status=passed`
  - `command` field includes a snippet that uses `ACCESS_TOKEN="$(... | jq -r <tokenJq>)"` and does not embed secrets

## run: suite auth via secret JSON (rest provider)
- Setup:
  - Suite includes `.auth` block with `provider=rest`, `secretEnv=API_TEST_AUTH_JSON`, and a `loginRequestTemplate`.
  - Environment provides `API_TEST_AUTH_JSON` as valid JSON containing credentials for at least one profile.
  - Multiple cases reference the same auth profile name (token/jwt field).
- Command: `API_TEST_AUTH_JSON='<json>' api-test run --suite smoke`
- Expect:
  - runner logs in once per profile and caches token
  - cases pass with injected `ACCESS_TOKEN` (but token is never printed)
  - failures include stable error messages like `auth_login_failed(...)` when credentials are missing/invalid

## run: cleanup steps (REST + GraphQL)
- Setup:
  - Suite contains a write-capable case with `allowWrite=true`.
  - Provide `--allow-writes` (or set env local) so execution and cleanup are allowed.
  - Case defines `cleanup`:
    - REST cleanup step deletes a resource using `pathTemplate` and `vars` extracted from the main response.
    - GraphQL cleanup step runs an operation with `varsTemplate` or `varsJq`.
- Command: `api-test run --suite smoke --allow-writes`
- Expect:
  - cleanup runs after the main case
  - if cleanup fails, the case becomes failed with `cleanup_failed` and suite exits `2`

## run: results JSON and artifacts contract
- Setup: any suite run that produces at least one executed case.
- Command: `api-test run --suite smoke --out out/api-test-runner/results.json --junit out/api-test-runner/junit.xml`
- Expect:
  - stdout JSON matches schema v1 and includes `suiteFile`, `runId`, `outputDir`, and per-case `stdoutFile`/`stderrFile`
  - `--out` file content matches stdout JSON exactly
  - JUnit file exists and contains testcase entries with durations

## summary: renders markdown from results JSON
- Setup:
  - Use results JSON from a prior run; include at least one failed and one skipped case.
- Command(s):
  - `api-test summary --in out/api-test-runner/results.json --slow 5 --out out/api-test-runner/summary.md`
  - `cat out/api-test-runner/results.json | api-test summary`
- Expect:
  - Markdown contains totals, run info, failed list, and slowest list
  - When `--hide-skipped` is set, skipped list is omitted
