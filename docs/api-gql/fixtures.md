# api-gql fixtures

These fixtures define deterministic scenarios for integration tests. Tests should use a local HTTP server and temporary
`setup/graphql` directories (do not rely on external network).

## call: query success (no vars)
- Setup:
  - Start a local GraphQL-like HTTP server that returns:
    - HTTP 200
    - JSON body with `{"data": {"health": {"ok": true}}}` for the provided query.
  - Create `setup/graphql/operations/health.graphql`.
- Command: `api-gql call --url http://127.0.0.1:<port>/graphql setup/graphql/operations/health.graphql`
- Expect:
  - exit `0`
  - stdout is JSON response body only

## call: query success with vars and min-limit bump
- Setup:
  - Variables JSON includes nested `limit` fields with values below 5.
  - Set `GQL_VARS_MIN_LIMIT=5`.
- Command: `api-gql call --url http://127.0.0.1:<port>/graphql op.graphql vars.json`
- Expect:
  - request uses a variables payload with all numeric `limit` fields bumped to at least 5

## call: missing operation file
- Setup: no file at the provided path.
- Command: `api-gql call --url http://127.0.0.1:<port>/graphql missing.graphql`
- Expect:
  - exit `1`
  - stderr contains `Operation file not found`

## call: JWT profile present
- Setup:
  - `setup/graphql/jwts.local.env` contains a JWT string for `GQL_JWT_DEFAULT`.
- Command: `api-gql call --jwt default --url http://127.0.0.1:<port>/graphql op.graphql`
- Expect:
  - request includes `Authorization: Bearer <token>`
  - JWT format validation runs by default (warn vs strict tested separately)

## call: JWT profile missing triggers auto-login fallback
- Setup:
  - Select `--jwt default`, but do not define `GQL_JWT_DEFAULT`.
  - Provide a login operation:
    - `setup/graphql/login.graphql` (or `setup/graphql/operations/login.graphql`)
    - optionally `login.variables.local.json`
  - Server returns a login response where `.data[loginRootField].accessToken` is set (or the root field is a string token).
- Command: `api-gql call --jwt default --url http://127.0.0.1:<port>/graphql op.graphql`
- Expect:
  - runner executes login first and extracts token
  - runner then executes the main operation using that token

## report: allow-empty gating
- Setup:
  - Generate a “draft” or empty response case.
- Command(s):
  - `api-gql report --case "Draft" --op op.graphql --out docs/draft.md` (no run/response)
  - `api-gql report --case "Draft" --op op.graphql --out docs/draft.md --allow-empty`
- Expect:
  - without `--allow-empty`: exit non-zero and no report is produced
  - with `--allow-empty`: report is produced

## list: env and JWT names
- Setup:
  - `setup/graphql/endpoints.env` contains multiple `GQL_URL_<ENV>` entries.
  - `setup/graphql/jwts.env` contains multiple `GQL_JWT_<NAME>` entries.
- Command(s):
  - `api-gql call --list-envs --config-dir setup/graphql`
  - `api-gql call --list-jwts --config-dir setup/graphql`
- Expect:
  - exit `0`
  - deterministic sorted output of available names

## schema: path resolution and cat
- Setup:
  - Configure schema via `setup/graphql/schema.env` (`GQL_SCHEMA_FILE=schema.gql`) or provide a fallback candidate.
- Command(s):
  - `api-gql schema --config-dir setup/graphql`
  - `api-gql schema --config-dir setup/graphql --cat`
- Expect:
  - default prints the resolved schema path
  - `--cat` prints file contents

## history: append and rotation
- Setup:
  - Run a successful call with `--config-dir setup/graphql`.
  - Force rotation via `GQL_HISTORY_MAX_MB` in a stress test.
- Command(s):
  - `api-gql history --config-dir setup/graphql --last`
  - `api-gql history --config-dir setup/graphql --command-only`
- Expect:
  - history entry starts with a metadata line beginning with `#`
  - command-only output omits metadata
  - token values are never present; only JWT profile names may appear

## mutation classification (suite runner dependency)
- Setup:
  - Operation file contains a `mutation` operation definition.
- Expect:
  - suite runner classifies the operation as write-capable and enforces allowWrite + writes-enabled guardrails.
