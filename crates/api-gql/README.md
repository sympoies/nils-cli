# api-gql parity spec

## Overview
`api-gql` is a GraphQL operation runner plus helper commands for history replay, Markdown report generation,
and schema file resolution.

Source-of-truth behavior is defined by the legacy Codex Kit scripts and report contract:
- `graphql-api-testing/scripts/gql.sh`
- `graphql-api-testing/scripts/gql-history.sh`
- `graphql-api-testing/scripts/gql-report.sh`
- `graphql-api-testing/scripts/gql-schema.sh`
- `graphql-api-testing/references/GRAPHQL_API_TEST_REPORT_CONTRACT.md`

Primary goals:
- Behavioral parity for flags, environment variables, list modes, exit codes, and history/report semantics.
- Deterministic, CI-friendly behavior (stable outputs; no secrets in logs by default).

## Entry point
Command: `api-gql <command> [args]`

Commands:
- `api-gql call ...` (default)
- `api-gql history ...`
- `api-gql report ...`
- `api-gql schema ...`

## call
Usage (parity intent):
`api-gql call [--env <name> | --url <url>] [--jwt <name>] [--config-dir <dir>] [--list-envs] [--list-jwts] [--no-history] <operation.graphql> [variables.json]`

### CLI surface
Flags and options:
- `-e, --env <name>`: endpoint preset name (resolved from config) OR a literal `http(s)://...` URL.
- `-u, --url <url>`: explicit GraphQL endpoint URL.
- `--jwt <name>`: JWT profile name (see “Auth selection + auto-login fallback”).
- `--config-dir <dir>`: seed directory used for setup-dir discovery (see below).
- `--list-envs`: list available endpoint preset names and exit `0`.
- `--list-jwts`: list available JWT profile names and exit `0`.
- `--no-history`: one-off disable of history writing for this invocation.
- `-h, --help`: print help and exit `0`.

Environment variables (call runner):
- `GQL_URL`: explicit GraphQL endpoint URL (overridden by `--env/--url`).
- `ACCESS_TOKEN`: if set (and no JWT profile is selected), send `Authorization: Bearer <token>`.
- `GQL_JWT_NAME`: default JWT profile name (same meaning as `--jwt`).
- `GQL_JWT_VALIDATE_ENABLED` (default `true`): enable best-effort JWT format + `exp`/`nbf` checks when a token is present.
- `GQL_JWT_VALIDATE_STRICT` (default `false`): fail hard on invalid/non-JWT token format instead of warning.
- `GQL_JWT_VALIDATE_LEEWAY_SECONDS` (default `0`): clock-skew allowance for `exp`/`nbf`.
- `GQL_VARS_MIN_LIMIT` (default `5`, `0` disables): bump numeric `limit` fields in variables JSON to at least this value.
- `GQL_HISTORY_ENABLED` (default `true`): enable/disable history writing.
- `GQL_HISTORY_FILE`: override history path (relative paths resolve under `<setup_dir>`).
- `GQL_HISTORY_LOG_URL_ENABLED` (default `true`): when `false`, omit URL values from history entries.
- `GQL_HISTORY_MAX_MB` (default `10`, `0` disables): rotate history when it reaches this size.
- `GQL_HISTORY_ROTATE_COUNT` (default `5`): number of rotated history files to keep.

Config files (setup dir):
- `endpoints.env` (and optional `endpoints.local.env` override):
  - `GQL_URL_<ENV>` variables define endpoint presets.
  - `GQL_ENV_DEFAULT` defines the default preset name used when neither `--env` nor `GQL_URL` is provided.
- `jwts.env` (and optional `jwts.local.env` override):
  - `GQL_JWT_<NAME>` variables define JWT token values for named profiles.
  - `GQL_JWT_NAME` can set the default profile name.

### Inputs
- Operation file: required, must exist.
- Variables file: optional.
  - In the legacy scripts, invalid JSON may fail later (HTTP client parsing, min-limit bumping, or curl payload build).
  - Parity intent for Rust: treat variables as JSON and fail early with a clear error if invalid.

### Endpoint selection
Ported from `gql.sh`:

Resolution order:
1. `--url <url>`
2. `--env <name>`:
   - if `<name>` looks like `http(s)://...`, treat it as a URL.
   - otherwise resolve from `setup/graphql/endpoints.env` (+ `endpoints.local.env`) as `GQL_URL_<ENV>`.
3. `GQL_URL=<url>` env var
4. `GQL_ENV_DEFAULT` from endpoints env (resolved as `GQL_URL_<ENV>`)
5. default: `http://localhost:6700/graphql`

### List modes (`--list-envs`, `--list-jwts`)
These are early-exit modes that do not require an operation file.

- `--list-envs`:
  - Requires `endpoints.env` to exist in the resolved setup dir.
  - Prints one env name per line, derived from variables matching `GQL_URL_<SUFFIX>=...`.
  - Output is lowercased and sorted unique.
- `--list-jwts`:
  - Requires at least one of `jwts.env` or `jwts.local.env` to exist in the resolved setup dir.
  - Prints one JWT profile name per line, derived from variables matching `GQL_JWT_<SUFFIX>=...` (excluding `<SUFFIX>=NAME`).
  - Output is lowercased and sorted unique.

### Setup dir discovery
Ported from `gql.sh`:
- Seed directory:
  - `--config-dir` if provided,
  - otherwise the operation file directory,
  - otherwise current working directory.
- Search upward for `endpoints.env`, `jwts.env`, or `jwts.local.env`.
- Fallback: if `setup/graphql` exists and no config files were found, treat it as the setup dir.

### Auth selection + auto-login fallback
Ported from `gql.sh`:

JWT profile selection is true if any of these are set:
- `--jwt <name>`
- `GQL_JWT_NAME=<name>`
- `GQL_JWT_NAME` in `setup/graphql/jwts.env` (+ local override)

When a JWT profile is selected:
- token is read from `GQL_JWT_<NAME>` in `jwts.env` / `jwts.local.env`.
- if selected but missing/empty, `gql.sh` attempts auto-login (best-effort, requires JSON tooling):
  1. Find a login operation file in the first matching directory:
     - `<setup_dir>/login.<profile>.graphql`, else `<setup_dir>/login.graphql`
     - `<setup_dir>/operations/login.<profile>.graphql`, else `<setup_dir>/operations/login.graphql`
     - `<setup_dir>/ops/login.<profile>.graphql`, else `<setup_dir>/ops/login.graphql`
  2. Select an optional variables file (first match):
     - `login.<profile>.variables.local.json`
     - `login.<profile>.variables.json`
     - `login.variables.local.json`
     - `login.variables.json`
  3. Execute the login operation against the resolved endpoint **without** any Authorization header.
  4. Determine the login root field:
     - Best-effort: first field name inside the first selection set `{ ... }` in the login operation text.
  5. Extract a token from the login response JSON:
     - If `.data[rootField]` is a non-empty string, that string is the token.
     - Otherwise find the first non-empty string value of `.accessToken` or `.token` anywhere under `.data[rootField]`.
  6. Guardrail: do not auto-login if the main operation file equals the login operation file (prevents recursion).

When a JWT profile is NOT selected:
- `ACCESS_TOKEN` is used as `Authorization: Bearer <token>` if set.

JWT validation:
- Enabled by default (`GQL_JWT_VALIDATE_ENABLED=true`).
- Checks token shape and (best-effort) `exp`/`nbf` timestamps (no signature verification).
- On invalid token format:
  - strict mode (`GQL_JWT_VALIDATE_STRICT=true`) fails hard.
  - non-strict mode warns and proceeds.
- Legacy degradation: if `python3` is missing, the script warns and skips JWT validation entirely.

### Variables min-limit normalization
Ported from `gql.sh`:
- If `GQL_VARS_MIN_LIMIT` is set and > 0 (default: `5`), any numeric `limit` fields in the variables JSON are bumped
  to at least that value (applies to nested objects, including pagination-style inputs).
- Setting `GQL_VARS_MIN_LIMIT=0` disables the behavior.
- Legacy degradation:
  - If neither `jq` nor `python3` exists, the script silently skips the transformation (uses original variables file).
  - If transformation tooling exists but parsing fails, the script fails the run.

### Operation + variables request shape
Parity intent mirrors the legacy scripts:
- HTTP method: `POST`
- Content-Type: `application/json`
- Authorization:
  - include `Authorization: Bearer <token>` only when a token is selected/present.
- Body:
  - Always includes the GraphQL operation text as `query`.
  - Includes `variables` only when a variables file was supplied.
  - Shape:
    - with vars: `{"query":"...","variables":{...}}`
    - without vars: `{"query":"..."}`

### Errors and `.errors` handling
- Non-2xx HTTP responses are treated as failures.
- GraphQL application errors under `.errors` are **not** automatically treated as failures by the legacy runner; callers
  (including CI) are expected to assert on `.errors` explicitly (for example by piping to `jq -e`).

### Output contract
- Stdout: response body only (typically JSON), with no additional decoration.
- Stderr: errors and guardrail messages.

### History behavior
Ported from `gql.sh` + `gql-history.sh`:
- Default history file: `<setup_dir>/.gql_history` (overridable via `GQL_HISTORY_FILE`).
- Disable history: `--no-history` or `GQL_HISTORY_ENABLED=false`.
- Rotation: when file exceeds `GQL_HISTORY_MAX_MB` (default 10; 0 disables), keep `GQL_HISTORY_ROTATE_COUNT` (default 5).
- Command snippets must never include secret token values; JWT profile names may be logged.
- `api-gql history` supports:
  - `--last` (default), `--tail <n>`, and `--command-only` (omit metadata lines starting with `#`).
- Legacy implementation details (parity-critical output shape, best-effort internals):
  - History is appended on process exit (success or failure) and records the exit code.
  - Entries are blank-line separated “paragraphs”:
    - line 1: metadata beginning with `#`, including `exit=<code>` and setup dir (often relative to the invocation dir).
    - subsequent lines: a copy/pasteable command snippet with line continuations and a trailing `| jq .`.
  - URL logging can be suppressed with `GQL_HISTORY_LOG_URL_ENABLED=false`:
    - metadata prints `url=<omitted>`.
    - the snippet omits the `--url ...` flag entirely.

### Exit codes
`api-gql call` (including list modes) uses these stable exit codes:
- `0`: success (request executed successfully OR list mode printed output).
- `1`: any error (invalid inputs/config, auth/login failure, JWT validation failure, network error, non-2xx HTTP status).

## report
Markdown report generator ported from `gql-report.sh`.

Usage (parity intent):
`api-gql report --case <name> --op <operation.graphql> [--vars <variables.json>] [--env <name> | --url <url>] [--jwt <name>] [--config-dir <dir>] [--run | --response <file|->] [--out <path>] [--allow-empty] [--no-redact] [--no-command] [--no-command-url] [--project-root <path>]`

### CLI surface
Flags and options:
- `--case <name>`: required case label (used in the report header + default filename slug).
- `--op, --operation <operation.graphql>`: required operation file path.
- `--vars, --variables <variables.json>`: optional variables JSON file path.
- `--run`: execute the operation via the runner and embed the response.
- `--response <file|->`: use a response from a file (or `-` for stdin) and embed it.
- `--out <path>`: output report path (defaults described below).
- `-e, --env <name>` / `-u, --url <url>` / `--jwt <name>` / `--config-dir <dir>`: passed through to the runner when `--run` is used.
- `--allow-empty` (alias `--expect-empty`): allow writing a report with no response, or a response that contains no data.
- `--no-redact` / `--redact`: control redaction (default: redact enabled).
- `--no-command`: omit the command snippet section.
- `--no-command-url`: when the snippet uses `--url`, omit the URL value (prints `<omitted>`).
- `--project-root <path>`: override project root used for default output path + relative path rendering.
- `-h, --help`: print help and exit `0`.

Environment variables (report generator):
- `GQL_REPORT_DIR`: default output directory when `--out` is not set.
  - If relative, it is resolved against `<project root>`.
  - Default: `<project root>/docs`.
- `GQL_ALLOW_EMPTY_ENABLED` (default `false`): same as `--allow-empty`.
- `GQL_VARS_MIN_LIMIT` (default `5`, `0` disables): same min-limit bumping semantics as the runner; report includes a note
  when bumping occurs.
- `GQL_REPORT_INCLUDE_COMMAND_ENABLED` (default `true`): if `false`, omit the command snippet (same as `--no-command`).
- `GQL_REPORT_COMMAND_LOG_URL_ENABLED` (default `true`): if `false`, omit the URL value from the snippet
  (same as `--no-command-url`).

### Output path defaults
When `--out` is not set:
- Determine `<project root>`:
  - Prefer git root (`git rev-parse --show-toplevel`); if unavailable, fall back to the current working directory.
  - (Or use `--project-root` when provided.)
- Determine output directory: `GQL_REPORT_DIR` if set, otherwise `<project root>/docs`.
- Output filename: `<YYYYMMDD-HHMM>-<slug(case)>-api-test-report.md`.

### Allow-empty gating (history/report semantics)
This is the key guardrail from `GRAPHQL_API_TEST_REPORT_CONTRACT.md` and `gql-report.sh`.

By default (no allow-empty):
- Refuse to write a report unless a “real response” is provided:
  - require `--run` OR `--response`.
- Refuse to write a report if the response is not valid JSON.
- Refuse to write a report if the response appears to contain “no data records”.

“No data records” is determined by a jq-based heuristic over `.data`:
- Walk scalar values under `.data`.
- Ignore common meta-only keys (case-insensitive): `__typename`, `pageInfo`, `totalCount`, `count`, `cursor`, `edges`,
  `nodes`, `hasNextPage`, `hasPreviousPage`, `startCursor`, `endCursor`.
- If there are zero remaining scalar values, treat the response as empty/no-data.

When allow-empty is enabled (`--allow-empty` or `GQL_ALLOW_EMPTY_ENABLED=true`), all of the above blocks are lifted.

### Redaction rules
Default behavior is secret-safe:
- When redact is enabled (default), redact:
  - `.accessToken`, `.refreshToken`, `.password` (any nesting) in both variables and response.
- Command snippets must never inline token values (only names like `--jwt <profile>` are allowed).

### Exit codes
`api-gql report` uses these stable exit codes:
- `0`: report written successfully; prints the report path to stdout.
- `1`: any error (invalid inputs, jq/assertion failure, allow-empty guardrail refusal, runner failure when using `--run`).

## schema
Schema resolver ported from `gql-schema.sh`.

Usage (parity intent):
`api-gql schema [--config-dir <dir>] [--file <path>] [--cat]`

### CLI surface
Flags and options:
- `--config-dir <dir>`: setup dir discovery seed (same semantics as `call`).
- `--file <path>`: explicit schema file path (overrides env + config).
- `--cat`: print schema file contents instead of the resolved path.
- `-h, --help`: print help and exit `0`.

Environment variables:
- `GQL_SCHEMA_FILE`: overrides schema file path (relative paths resolve under `<setup_dir>`).

Config files (recommended):
- `schema.env`: committed; sets `GQL_SCHEMA_FILE`.
- `schema.local.env`: local override; sets `GQL_SCHEMA_FILE`.

Resolution order:
1. `--file <path>` (explicit override)
2. `GQL_SCHEMA_FILE` env var
3. `GQL_SCHEMA_FILE` from `<setup_dir>/schema.local.env` then `<setup_dir>/schema.env`
4. Fallback candidates under `<setup_dir>`:
   `schema.gql`, `schema.graphql`, `schema.graphqls`, `api.graphql`, `api.gql`

Output:
- Default: print resolved schema file path.
- With `--cat`: print schema file contents.

### Exit codes
`api-gql schema` uses these stable exit codes:
- `0`: resolved successfully (path printed or file contents printed).
- `1`: any error (setup dir cannot be resolved, schema not configured, schema file missing, invalid CLI args).

## history
History replay helper ported from `gql-history.sh`.

Usage (parity intent):
`api-gql history [--config-dir <dir>] [--file <path>] [--last | --tail <n>] [--command-only]`

### CLI surface
Flags and options:
- `--config-dir <dir>`: setup dir discovery seed (default: current working directory).
- `--file <path>`: explicit history file path (default: `<setup_dir>/.gql_history`).
- `--last`: print the last entry (default).
- `--tail <n>`: print the last N entries (blank-line separated).
- `--command-only`: omit the metadata line (starting with `#`) from each entry.
- `-h, --help`: print help and exit `0`.

Environment variables:
- `GQL_HISTORY_FILE`: override history file path (relative paths resolve under `<setup_dir>`).

### Output contract
- Output is one or more history “entries” separated by a blank line.
- With `--command-only`, the first metadata line is omitted (when present).

### Exit codes
Parity intent mirrors the legacy script behavior:
- `0`: success; printed one or more entries.
- `1`: any error (invalid args, cannot resolve setup dir, file missing).
- `3`: history file exists but contains zero entries (awk RS= behavior).

## Mutation detection (write-capable classification)
Suite runner safety relies on detecting `mutation` operation definitions.
Parity intent mirrors the suite runner’s legacy implementation (`api-test.sh`) and is **best-effort**:

Detection algorithm:
1. Read the operation file as text (UTF-8).
2. Strip block comments: `/* ... */` (best-effort).
3. Strip GraphQL string literals:
   - triple-quoted strings `"""..."""` (best-effort),
   - double-quoted strings `"..."` with escapes.
4. Strip line comments:
   - GraphQL `# ...`
   - and also `// ...` (some tools allow it).
5. Regex search (case-insensitive, multiline) for an operation definition line:
   - `^\s*mutation\b(?=\s*(?:\(|@|\{|[_A-Za-z]))`
   - The lookahead intentionally excludes schema shorthand like `mutation: Mutation` (because `:` does not match).

Semantics:
- If the regex matches anywhere, the operation is classified as **write-capable**.
- Write-capable classification is used by the suite runner guardrails:
  - If a case is a mutation and `allowWrite` is not `true`, the case fails with a stable reason.
  - If `allowWrite=true` but writes are not enabled for the run (and env is not `local`), the case is skipped.
- `api-gql` itself does not block executing mutations; it is a low-level runner.

## External dependencies (inventory + policy)
This section is an explicit inventory of the legacy external dependencies plus the chosen Rust-port policy.

### Legacy scripts: external dependencies
- HTTP client:
  - required: one of `xh`, `http` (HTTPie), or `curl`
  - `curl` path additionally requires `jq` to build the JSON payload
- JSON tooling:
  - `jq`:
    - required by `gql-report.sh`
    - required for auto-login token extraction in `gql.sh`
    - used for variables min-limit bumping when present
  - `python3`:
    - optional: JWT validation in `gql.sh` (skipped when missing)
    - optional: variables min-limit bumping fallback in `gql.sh` when `jq` is missing
- Git (optional):
  - `gql-report.sh` uses `git rev-parse --show-toplevel` to find `<project root>`, but falls back to `pwd` if git is missing.
- Standard UNIX utilities (assumed available in the legacy environment):
  - `awk` (login root-field extraction), `sed`, `tr`, `sort`, `wc`, `date`, `mktemp`, `mv`, `mkdir`, `cat`.

### Rust port policy (api-gql binary)
Goal: remove runtime reliance on external tools while preserving user-facing behavior.

- HTTP client (`xh/http/curl`): eliminate; implement HTTP requests directly in Rust.
- JSON tooling (`jq`): eliminate by default; implement:
  - request payload construction,
  - variables min-limit bumping,
  - report formatting + redaction,
  - “meaningful data” allow-empty heuristic.
  Optional compat policy: only shell out to `jq` if an explicit “compat mode” is enabled and documented.
- `python3`: eliminate; implement JWT checks and mutation detection in Rust.
- `git`: eliminate; implement “project root” resolution by searching upward for `.git/` (fallback to cwd).
- Coreutils (`date/wc/mv/mkdir/mktemp/...`): eliminate; use Rust stdlib for filesystem and timestamps.

If any optional external tool invocation remains (compat mode), its behavior must be explicitly documented in `--help` and
covered by deterministic tests.
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
