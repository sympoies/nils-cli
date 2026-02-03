# api-gql parity spec

## Overview
`api-gql` is a GraphQL operation runner that executes a `.graphql` file (and optional variables) and prints the response
body to stdout. It is a Rust port of the Codex Kit GraphQL scripts:
- `graphql-api-testing/scripts/gql.sh` (call)
- `graphql-api-testing/scripts/gql-history.sh` (history)
- `graphql-api-testing/scripts/gql-report.sh` (report)
- `graphql-api-testing/scripts/gql-schema.sh` (schema resolver)

Parity-critical outcomes:
- The CLI surface (flags, env vars, defaults, help intent).
- Exit code contract.
- Endpoint + auth selection rules (including auto-login fallback).
- JWT validation behavior (format + `exp`/`nbf` time checks).
- History + report artifacts (file locations, record/report structure, redaction defaults).

See also: `crates/api-testing-core/README.md`.

## CLI surface

### Entry point and commands
Command: `api-gql <command> [args]`

Commands (parity mapping):
- `api-gql call` (default when no subcommand is given) -> `gql.sh`
- `api-gql history` -> `gql-history.sh`
- `api-gql report` -> `gql-report.sh`
- `api-gql report-from-cmd` -> helper: turn a command snippet into a report
- `api-gql schema` -> `gql-schema.sh`

Help intent:
- `api-gql --help` explains the commands and config discovery (`--config-dir`).
- Each subcommand help lists relevant flags and environment variables.

### Flags (by command)

#### `api-gql call` (default)
Parity intent usage:
`api-gql call [--env <name> | --url <url>] [--jwt <name>] [--config-dir <dir>] [--list-envs] [--list-jwts] [--no-history] <operation.graphql> [variables.json]`

Flags:
- `-e, --env <name>`: select an endpoint preset or literal URL.
- `-u, --url <url>`: explicit GraphQL endpoint URL (highest precedence).
- `--jwt <name>`: JWT profile name (see Auth selection + auto-login).
- `--config-dir <dir>`: seed setup-dir discovery (or pin it when no known files are present).
- `--list-envs`: list available endpoint preset names (from `endpoints.env`) and exit `0`.
- `--list-jwts`: list available JWT profile names (from `jwts(.local).env`) and exit `0`.
- `--no-history`: disable writing to `.gql_history` for this run only.
- `-h, --help`: print help and exit `0`.

List modes:
- `--list-envs` prints lowercased, sorted, de-duplicated env names (from `GQL_URL_*` in endpoints files).
- `--list-jwts` prints lowercased, sorted, de-duplicated JWT profile names (from `GQL_JWT_*`, excluding `GQL_JWT_NAME`).
- `--list-jwts` errors if neither `jwts.env` nor `jwts.local.env` exists.

#### `api-gql history`
Parity intent usage:
`api-gql history [--config-dir <dir>] [--file <path>] [--last | --tail <n>] [--command-only]`

Flags:
- `--config-dir <dir>`: seed setup-dir discovery for the history file.
- `--file <path>`: explicit history file path (relative paths are resolved against the setup dir).
- `--last`: print the last entry (default).
- `--tail <n>`: print the last `n` entries.
- `--command-only`: omit the metadata line (lines starting with `#`) from each entry.
- `-h, --help`: print help and exit `0`.

#### `api-gql report`
Parity intent usage:
`api-gql report --case <name> --op <operation.graphql> [--vars <variables.json>] [--run | --response <file|->] [options]`

Core options:
- `--case <name>`: required human label for the report.
- `--op, --operation <file>`: required operation file path.
- `--vars, --variables <file>`: optional variables JSON file path.
- `--run`: execute the operation via `api-gql call` and embed the response.
- `--response <file|->`: embed an existing response (use `-` for stdin).
- `--out <path>`: output report path (default described below).
- `--allow-empty` (alias `--expect-empty`): allow writing a report without data records.
- `--no-redact`: disable secret redaction in variables/response JSON.
- `--no-command`: omit the command snippet section.
- `--no-command-url`: when `--url` is used, omit the URL value in the command snippet.
- `--project-root <path>`: override the project root used for default output path + relative paths.
- `--config-dir <dir>`: passed through to `api-gql call` (setup dir selection).

Endpoint/auth pass-through options:
- `-e, --env <name>`
- `-u, --url <url>`
- `--jwt <name>`

Mutual exclusivity rules (parity):
- `--run` and `--response` MUST NOT be used together (error).
- At least one of `--run` or `--response` MUST be provided (error).

#### `api-gql report-from-cmd`
Usage:
`api-gql report-from-cmd [--case <name>] [--out <path>] [--response <file|->] [--allow-empty] [--dry-run] [--stdin] <snippet>`

Flags:
- `--case <name>`: override the derived case name.
- `--out <path>`: output report path.
- `--response <file|->`: embed an existing response (use `-` for stdin).
- `--allow-empty` (alias `--expect-empty`): allow writing a report without data records.
- `--dry-run`: print the equivalent `api-gql report ...` command and exit `0`.
- `--stdin`: read the snippet from stdin (cannot be used with `--response -`).

#### `api-gql schema`
Parity intent usage:
`api-gql schema [--config-dir <dir>] [--file <path>] [--cat]`

Flags:
- `--config-dir <dir>`: seed setup-dir discovery (same semantics as `call`).
- `--file <path>`: explicit schema file path (overrides env + schema.env).
- `--cat`: print schema file contents (default is to print the resolved path).
- `-h, --help`: print help and exit `0`.

## Environment variables (by function)

### Endpoint selection
- `GQL_URL=<url>`
  - Used only when neither `--env` nor `--url` is provided.
  - Overridden by `--env` / `--url`.
- `GQL_ENV_DEFAULT` (file-based; see endpoints files)
  - Read from `endpoints.env` / `endpoints.local.env` (not from the process environment).

### Auth selection
- `ACCESS_TOKEN=<token>`
  - Used only when no JWT profile is selected.
  - When present, sends `Authorization: Bearer <token>`.
- `GQL_JWT_NAME=<name>`
  - Selects a JWT profile name (equivalent to `--jwt <name>`).
  - Presence of this variable counts as “JWT profile selected” (affects `ACCESS_TOKEN` fallback rules).

### JWT validation controls
These apply only when a bearer token is present:
- `GQL_JWT_VALIDATE_ENABLED=true|false` (default: `true`)
- `GQL_JWT_VALIDATE_STRICT=true|false` (default: `false`)
- `GQL_JWT_VALIDATE_LEEWAY_SECONDS=<int>` (default: `0`, clamped to `>= 0`)

Boolean parsing parity:
- Values are trimmed and lowercased.
- Only `true` and `false` are accepted.
- Any other value prints a warning to stderr and is treated as `false`.

### Variables min-limit normalization
- `GQL_VARS_MIN_LIMIT=<int>` (default: `5`, `0` disables)
  - Bumps numeric `limit` fields in variables JSON to at least this value.

### History controls (call)
- `GQL_HISTORY_ENABLED=true|false` (default: `true`)
- `GQL_HISTORY_FILE=<path>` (default: `<setup_dir>/.gql_history`; relative paths resolve under setup dir)
- `GQL_HISTORY_LOG_URL_ENABLED=true|false` (default: `true`)
- `GQL_HISTORY_MAX_MB=<int>` (default: `10`; `0` disables rotation; clamped to `>= 0`)
- `GQL_HISTORY_ROTATE_COUNT=<int>` (default: `5`; clamped to `>= 1`)

### Report controls
- `GQL_REPORT_DIR=<path>`
  - Default directory for generated reports when `--out` is not set.
  - If relative, it is resolved against `<project_root>`.
  - Default: `<project_root>/docs`.
- `GQL_REPORT_INCLUDE_COMMAND_ENABLED=true|false` (default: `true`)
  - If `false`, omits the command snippet section (equivalent to `--no-command`).
- `GQL_REPORT_COMMAND_LOG_URL_ENABLED=true|false` (default: `true`)
  - If `false`, omits the URL value in the command snippet when `--url` is used (equivalent to `--no-command-url`).
- `GQL_ALLOW_EMPTY_ENABLED=true|false` (default: `false`)
  - Allows report generation with empty/no-data responses (equivalent to `--allow-empty`).

## Setup/config discovery

### Setup dir definition
The “setup dir” is the directory that contains GraphQL config files. Canonical location: `setup/graphql`.

The setup dir influences:
- Endpoint preset resolution (`endpoints.env` + `endpoints.local.env`)
- JWT profile resolution (`jwts.env` + `jwts.local.env`)
- History file location (`<setup_dir>/.gql_history`)
- Schema resolution (`schema.env` / `schema.local.env`)

### File parsing rules (endpoints/jwts env files)
The legacy scripts parse `.env`-like files with these rules:
- Blank lines and lines starting with `#` are ignored.
- Lines may be `KEY=VALUE` or `export KEY=VALUE`.
- Values may be wrapped in single or double quotes (quotes are stripped).
- If a key is assigned multiple times, the last assignment wins.
- Local override files are read after the base file, so the local value wins.

Important parity quirks:
- `endpoints.local.env` is only consulted when `endpoints.env` exists.
  - A repo with only `endpoints.local.env` will not support `--env` selection.
- `jwts.local.env` may exist without `jwts.env` and is still used.

### Setup dir discovery: `api-gql call`
Parity algorithm (ported from `gql.sh`):
1. Seed directory:
   - If `--config-dir` is set: seed = `--config-dir`.
   - Else if an operation file is provided: seed = operation file directory.
   - Else: seed = current directory.
2. Search upward from the seed directory for the first matching file, in order:
   - `endpoints.env`
   - `jwts.env`
   - `jwts.local.env`
   If found: `setup_dir` is the directory containing that file.
3. Else, if `--config-dir` was explicitly set: use the seed directory as `setup_dir`.
4. Else, if `<invocation_dir>/setup/graphql` exists: use it.
5. Else: use the seed directory.

### Setup dir discovery: `api-gql history`
Parity algorithm (ported from `gql-history.sh`):
1. Seed directory:
   - If `--config-dir` is set: seed = `--config-dir`.
   - Else: seed = current directory.
2. Search upward from the seed directory for the first matching file, in order:
   - `.gql_history`
   - `endpoints.env`
   - `jwts.env`
   - `jwts.local.env`
   If found: `setup_dir` is the directory containing that file.
3. Else, if `<invocation_dir>/setup/graphql` exists: use it.
4. Else: use the seed directory.

### Setup dir discovery: `api-gql schema`
Parity algorithm (ported from `gql-schema.sh`):
1. Seed directory:
   - If `--config-dir` is set: seed = `--config-dir`.
   - Else: seed = current directory.
2. Search upward from the seed directory for the first matching file, in order:
   - `schema.env`
   - `schema.local.env`
   - `endpoints.env`
   - `jwts.env`
   - `jwts.local.env`
   If found: `setup_dir` is the directory containing that file.
3. Else, if `<invocation_dir>/setup/graphql` exists: use it.
4. Else: use the seed directory.

## Endpoint selection rules
Ported from `gql.sh`.

Endpoint resolution order (highest precedence first):
1. `--url <url>`
2. `--env <name>`
   - If `<name>` looks like a URL (`^https?://`), treat it as a URL (equivalent to `--url`).
   - Otherwise, resolve from `endpoints.env` (+ optional `endpoints.local.env`) using:
     - `<ENV_KEY> = uppercased(<name>) with non-alphanumerics replaced by '_'` (trim/collapse underscores)
     - URL = `GQL_URL_<ENV_KEY>`
3. `GQL_URL=<url>` environment variable
4. `GQL_ENV_DEFAULT` (from endpoints files) resolved as `GQL_URL_<ENV_KEY>`
5. Default: `http://localhost:6700/graphql`

If `--env <name>` is used (and is not a URL) but `endpoints.env` cannot be found under the setup dir:
- Fail with a clear error.

If `--env <name>` is unknown:
- Fail with a clear error listing available env presets discovered from `endpoints.env` (+ `endpoints.local.env` if present).

## Auth selection + auto-login
Ported from `gql.sh`.

### JWT profile selection
A “JWT profile is selected” if any of these sources provides a profile name:
- CLI: `--jwt <name>`
- Environment: `GQL_JWT_NAME=<name>`
- JWT files: `GQL_JWT_NAME=<name>` in `jwts.env` or `jwts.local.env`

Profile name normalization:
- The selected name is trimmed and lowercased for display/logging.
- When looking up the token variable, the name is converted to an env key (`<NAME_KEY>`) by uppercasing and converting
  non-alphanumerics to `_`.
  - Example: `local-dev` -> `LOCAL_DEV` -> lookup key `GQL_JWT_LOCAL_DEV`.

### Token source resolution
If a JWT profile is selected:
- Read the bearer token from `jwts.env` / `jwts.local.env` using `GQL_JWT_<NAME_KEY>`.
- If the selected token is empty or missing, attempt auto-login (see below).
- If auto-login is not configured or fails, the call fails.

If a JWT profile is NOT selected:
- If `ACCESS_TOKEN` is set: use it as the bearer token.
- Else: no `Authorization` header is sent.

### Authorization header behavior
When a bearer token is selected (from either source):
- Send `Authorization: Bearer <token>`.

### Auto-login fallback (when selected profile has no token)
The Rust port implements best-effort auto-login parity with `gql.sh`:
1. Find a login operation file (first match, lowercased profile name):
   - `<setup_dir>/login.<profile>.graphql`, else `<setup_dir>/login.graphql`
   - `<setup_dir>/operations/login.<profile>.graphql`, else `<setup_dir>/operations/login.graphql`
   - `<setup_dir>/ops/login.<profile>.graphql`, else `<setup_dir>/ops/login.graphql`
2. Select an optional variables file in the *same directory as the login op* (first match):
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

## JWT validation behavior
Ported from the legacy behavior, implemented in Rust.

Scope:
- Validates JWT shape and time-based claims only (no signature verification).
- Validation runs only when a bearer token is present.
- Validation is enabled by default and can be disabled with `GQL_JWT_VALIDATE_ENABLED=false`.

Time checks:
- `exp` (if present) must be parseable as a number. If `exp < now - leeway`, the token is treated as expired.
- `nbf` (if present) must be parseable as a number. If `nbf > now + leeway`, the token is treated as not-yet-valid.
- `GQL_JWT_VALIDATE_LEEWAY_SECONDS` is an integer number of seconds to allow for clock skew (default `0`).

Format checks:
- Token must be three dot-separated segments.
- Header and payload must be base64url-decodable JSON.
- Invalid formats are handled by the strictness policy below.

Failure policy:
- If the token is expired (`exp`) or not-yet-valid (`nbf`): always fail the request (exit non-zero).
- If the token is not a valid JWT (wrong segment count / decode / JSON / invalid claim types):
  - Strict mode (`GQL_JWT_VALIDATE_STRICT=true`) fails the request.
  - Non-strict mode prints a warning and proceeds (skipping further format validation).

## Variables min-limit normalization
Ported from `gql.sh`.

- If `GQL_VARS_MIN_LIMIT` is set and > 0 (default: `5`), any numeric `limit` fields in the variables JSON are bumped
  to at least that value (applies to nested objects; key name is case-sensitive).
- Setting `GQL_VARS_MIN_LIMIT=0` disables the behavior.
- Variables must be valid JSON; invalid JSON fails the run with a clear error.

## Operation execution & output semantics
Ported from `gql.sh`.

Request:
- HTTP method: `POST`
- Headers:
  - `Accept: application/json`
  - `Content-Type: application/json`
  - `Authorization: Bearer <token>` only when a token is selected
- Body:
  - Always includes the GraphQL operation text as `query`.
  - Includes `variables` only when a variables file was supplied.
  - Shapes:
    - with vars: `{"query":"...","variables":{...}}`
    - without vars: `{"query":"..."}`

Response handling:
- Non-2xx HTTP responses are treated as failures (exit `1`).
- GraphQL application errors under `.errors` do not affect the exit code.

Stdout/stderr contract:
- Stdout: response body only (typically JSON), with no decoration.
- Stderr: errors and warnings.

## History semantics
Ported from `gql.sh` + `gql-history.sh`.

### When history is written
- History is appended only for `api-gql call` (not for `history` or `report`).
- History is enabled by default and may be disabled by:
  - `--no-history` (for that run), or
  - `GQL_HISTORY_ENABLED=false`
- History append happens on process exit (including failure exits) when enabled.

### History file location
Default: `<setup_dir>/.gql_history`

Overrides:
- If `GQL_HISTORY_FILE` is set:
  - absolute paths are used as-is
  - relative paths are resolved against `<setup_dir>`

### Locking and best-effort writes
The history writer uses a lock directory (`<history_file>.lock`) to avoid concurrent writes.
If the lock cannot be acquired, history is skipped silently (no error).

### Rotation
If `GQL_HISTORY_MAX_MB > 0` and the history file size is `>= GQL_HISTORY_MAX_MB * 1024 * 1024`:
- Rotate the file to `.1`, `.2`, ..., keeping `GQL_HISTORY_ROTATE_COUNT` files.
- Rotation is best-effort; failures do not fail the request.

### Record format
Each entry is a blank-line-separated record:
1. A metadata line starting with `#`.
2. A copy/paste command snippet.
3. A trailing blank line separating records.

Metadata line shape (parity intent):
`# <timestamp> exit=<code> setup_dir=<rel> [env=<name> | url=<url>|url=<omitted>] [jwt=<name> | token=ACCESS_TOKEN]`

Details:
- Timestamp format is `YYYY-MM-DDTHH:MM:SS%z` (for example `2026-01-31T12:34:56-0800`).
- `setup_dir=<rel>` is rendered relative to the invocation directory when possible.
- URL logging is controlled by `GQL_HISTORY_LOG_URL_ENABLED`:
  - If `false` and the endpoint is URL-based, metadata uses `url=<omitted>` and the command snippet omits `--url`.
- Tokens are never logged by value:
  - JWT profile names may be logged (`jwt=<name>`).
  - When `ACCESS_TOKEN` is used (and no JWT profile is selected), metadata uses `token=ACCESS_TOKEN`.

Command snippet shape (parity intent):
- Multi-line with `\` continuations.
- Includes `--env`/`--url` and `--jwt` when applicable.
- Ends with `| jq .` for human-friendly JSON formatting.

Note on Rust port output:
- The legacy history snippet uses a `$CODEX_HOME/.../gql.sh` path when available.
- The Rust port emits an equivalent `api-gql` invocation instead, while preserving option semantics.

### `api-gql history` output rules
- Entries are parsed as blank-line-separated records.
- `--last` prints the last record.
- `--tail <n>` prints the last `n` records.
- `--command-only` drops the metadata line if it begins with `#`.
- Output includes a blank line after each printed record (including the last).

## Report semantics
Ported from `gql-report.sh` and aligned with `graphql-api-testing/references/GRAPHQL_API_TEST_REPORT_CONTRACT.md`.

### Output path and printing behavior
On success, the command prints the report path to stdout and exits `0`.

Default output path when `--out` is not set:
- `stamp = YYYYMMDD-HHMM` (local time)
- `case_slug = lowercased(case) with non-alphanumerics replaced by '-'` (trim/collapse hyphens; fallback `"case"`)
- `report_dir = GQL_REPORT_DIR` if set, else `<project_root>/docs`
  - if `GQL_REPORT_DIR` is relative, it is resolved against `<project_root>`
- `out = <report_dir>/<stamp>-<case_slug>-api-test-report.md`

Project root resolution:
- Default: the Git repo root (if available); otherwise the current directory.
- Override with `--project-root`.

### Response sourcing (`--run` vs `--response`)
`--run`:
- Executes `api-gql call` using the provided endpoint/auth/config options.
- If the call fails, the report is not written and the command exits non-zero.
- Records `Result: PASS` when the call exit code is `0`.

`--response <file|->`:
- Reads response bytes from the given file (or stdin).
- Does not execute the request.
- Records `Result: (response provided; request not executed)`.

### Allow-empty gating (no-data guardrail)
By default (no allow-empty):
- Refuses to write a report if the response is not valid JSON.
- Refuses to write a report if the response appears to contain no data records.

“No data records” is determined by a heuristic over `.data`:
- Walk scalar values under `.data`.
- Ignore common meta-only keys (case-insensitive):
  - `__typename`, `pageInfo`, `totalCount`, `count`, `cursor`, `edges`, `nodes`,
    `hasNextPage`, `hasPreviousPage`, `startCursor`, `endCursor`.
- If there are zero remaining scalar values, treat the response as empty/no-data.

When allow-empty is enabled (`--allow-empty` or `GQL_ALLOW_EMPTY_ENABLED=true`), all of the above blocks are lifted.

### Report Markdown structure
The report is Markdown and includes (parity intent):
- `# API Test Report (<YYYY-MM-DD>)`
- `## Test Case: <case>`
- Optional `## Command` section with a fenced `bash` block
- `Generated at: <timestamp-with-timezone>`
- Endpoint note:
  - `Endpoint: --url <url>` OR `Endpoint: --env <name>` OR `Endpoint: (implicit; see GQL_URL / GQL_ENV_DEFAULT)`
- Result note (`PASS` / provided)
- `### GraphQL Operation` (fenced `graphql` block)
- `### GraphQL Operation (Variables)` (fenced `json` block; `{}` when no vars file)
- `### Response` (fenced `json` or `text` block)
- Optional variables note when min-limit bumping occurs

### Redaction rules
Redaction is ON by default (for JSON formatting only).
When enabled, any object field with these keys is replaced with `<REDACTED>` (deep traversal, case-insensitive):
- `accessToken`, `access_token`
- `refreshToken`, `refresh_token`
- `password`
- `token`
- `apiKey`, `api_key`
- `authorization`
- `cookie`
- `set-cookie`

Notes:
- Redaction applies only when the variables/response is parseable as JSON.
- When the response is non-JSON text, it is included verbatim (no redaction).

### Command snippet inclusion and URL elision
Command snippet inclusion:
- Default: included.
- Omitted when:
  - `--no-command`, or
  - `GQL_REPORT_INCLUDE_COMMAND_ENABLED=false`

URL in command snippet (when `--url` is used):
- Default: included.
- When disabled (`--no-command-url` or `GQL_REPORT_COMMAND_LOG_URL_ENABLED=false`):
  - the URL value is replaced with `<omitted>`.

## Report-from-cmd semantics
`api-gql report-from-cmd` turns a call snippet into a report command.

Snippet parsing rules (best-effort parity with history snippets):
- Accepts `api-gql ...` and legacy `gql.sh ...` call snippets.
- Supports line continuations (`\` + newline) and truncates at the first pipe (`|`).
- Expands `$VARS` and `${VARS}` best-effort for tokenization.
- Ignores flags that do not affect report generation (`--no-history`, `--list-envs`, `--list-jwts`).

Case name derivation:
- Default case is derived from the operation filename stem plus metadata:
  - `"<op-stem> (<env-or-url-or-implicit>, jwt:<name>)"` (JWT suffix only if provided).

Execution modes:
- If `--dry-run` is set, prints the equivalent `api-gql report ...` command and exits `0`.
- Otherwise, delegates to `api-gql report` (using `--run` unless `--response` is provided).
- `--stdin` reads the snippet from stdin; it cannot be used with `--response -`.

## Schema semantics
Ported from `gql-schema.sh`.

Resolution order:
1. `--file <path>` (explicit override)
2. `GQL_SCHEMA_FILE` environment variable
3. `GQL_SCHEMA_FILE` from `<setup_dir>/schema.local.env` then `<setup_dir>/schema.env`
4. Fallback candidates under `<setup_dir>`:
   `schema.gql`, `schema.graphql`, `schema.graphqls`, `api.graphql`, `api.gql`

Notes:
- If a schema path is relative, it is resolved under `<setup_dir>`.
- If no schema is configured or the file does not exist, the command fails.

Output:
- Default: print the resolved schema file path.
- With `--cat`: print schema file contents.

## Mutation detection (suite runner dependency)
Suite runner safety relies on detecting `mutation` operation definitions.
Parity intent mirrors the legacy implementation and is best-effort:

Detection algorithm:
1. Read the operation file as text (UTF-8).
2. Strip block comments (`/* ... */`) best-effort.
3. Strip GraphQL string literals:
   - triple-quoted strings `"""..."""`
   - double-quoted strings `"..."` (with escapes)
4. Strip line comments:
   - GraphQL `# ...`
   - `// ...` (some tools allow it).
5. Scan lines for an operation definition beginning with `mutation`:
   - `^\s*mutation\b` followed by `(`, `@`, `{`, `_`, or a letter.
   - This intentionally excludes schema shorthand like `mutation: Mutation`.

Semantics:
- If the pattern matches anywhere, the operation is classified as **write-capable**.
- This classification is used by the suite runner guardrails (not by `api-gql` itself).

## Exit codes

### `api-gql call`
- `0`: request executed successfully.
- `1`: invalid input/config, auth/login failure, JWT validation failure, network error, or non-2xx HTTP status.

### `api-gql history`
- `0`: printed at least one record successfully.
- `1`: invalid arguments, setup/history discovery failure, or history file missing.
- `3`: history file exists but contains zero records (parity with legacy awk-based implementation).

### `api-gql report`
- `0`: report written and the output path printed.
- `1`: invalid arguments, missing files, refusal by allow-empty guardrails, runner failure when using `--run`,
  or failure to write report.

### `api-gql report-from-cmd`
- `0`: dry-run output printed or report written successfully.
- `1`: invalid snippet/flags, or underlying report failure.

### `api-gql schema`
- `0`: resolved successfully (path printed or file contents printed).
- `1`: setup dir cannot be resolved, schema not configured, schema file missing, or invalid CLI args.

## External dependencies (inventory + policy)

### Legacy script inventory (observed)
| Tool / runtime | Used by | Purpose | Legacy status |
| --- | --- | --- | --- |
| `curl` / `http` / `xh` | `gql.sh` | HTTP execution | required |
| `jq` | `gql.sh`, `gql-report.sh` | JSON parsing + redaction + min-limit | required |
| `python3` | `gql.sh` | JWT validation | optional |
| `git` | `gql-report.sh` | detect project root | optional |
| `awk` | `gql-history.sh` | record parsing (`RS=`) | required |
| coreutils (`date`, `mktemp`, `wc`, `mv`, `mkdir`, `head`, etc.) | all | plumbing | required |

### Rust port policy (api-gql)
Goal: eliminate runtime dependencies on external binaries for core behavior.

| Dependency | Policy | Rationale |
| --- | --- | --- |
| `curl` / `http` / `xh` | eliminate | implement HTTP client in Rust for portability and consistent error handling |
| `jq` | eliminate (default) | implement JSON parsing, redaction, and min-limit normalization in Rust |
| `python3` | eliminate | implement JWT parsing/time checks in Rust |
| `git` | eliminate | detect repo root in Rust (fallback to CWD) |
| shell utilities (`awk`, `date`, `mktemp`, etc.) | eliminate | implement history/report generation and file ops in Rust |

Note:
- History/report command snippets still end with `| jq .` for human-friendly formatting, but `api-gql` does not
  require `jq` at runtime.

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
  - Generate a "draft" or empty response case.
- Command(s):
  - `api-gql report --case "Draft" --op op.graphql --response resp.json`
  - `api-gql report --case "Draft" --op op.graphql --response resp.json --allow-empty`
- Expect:
  - without `--allow-empty`: exit non-zero and no report is produced
  - with `--allow-empty`: report is produced

## report: report-from-cmd dry run
- Setup:
  - A call snippet (from history or manual): `api-gql call --env local ops/health.graphql`.
- Command: `api-gql report-from-cmd --dry-run "api-gql call --env local ops/health.graphql"`
- Expect:
  - exit `0`
  - stdout contains `api-gql report --case` and references the operation

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
