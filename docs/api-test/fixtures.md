# api-test fixtures

These fixtures define deterministic scenarios for end-to-end tests of the suite runner and summary renderer.
Tests should use a local HTTP server that can serve both REST and GraphQL endpoints and should build temporary directory
layouts under a temp “repo root” (including a `.git` directory if the runner requires it).

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
