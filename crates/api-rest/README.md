# api-rest parity spec

## Overview
`api-rest` is a REST API runner that executes a JSON-defined request file and prints the HTTP response body to stdout.
It is a Rust port of the Codex Kit REST testing scripts:
- `rest-api-testing/scripts/rest.sh` (request runner; “call”)
- `rest-api-testing/scripts/rest-history.sh` (history viewer)
- `rest-api-testing/scripts/rest-report.sh` (Markdown report generator)

Parity-critical outcomes:
- The CLI surface (flags, env vars, defaults, help intent).
- Exit code contract.
- Request JSON schema validation rules (including `expect` and `cleanup`).
- Endpoint + auth selection rules (including token profile and `ACCESS_TOKEN` fallback).
- JWT validation behavior (format + `exp`/`nbf` time checks).
- History + report artifacts (file locations, record/report structure, redaction defaults).

See also: `crates/api-testing-core/README.md`.

## CLI surface

### Entry point and commands
Command: `api-rest <command> [args]`

Commands (parity mapping):
- `api-rest call` (default when no subcommand is given) → `rest.sh`
- `api-rest history` → `rest-history.sh`
- `api-rest report` → `rest-report.sh`

Help intent:
- `api-rest --help` explains the three commands and how config discovery works (`--config-dir`).
- Each subcommand help lists relevant flags and environment variables (grouped by purpose).
- Help text must clearly state that request files are JSON-only, and that `expect` turns the request into a CI/E2E contract.

### Flags (by command)

#### `api-rest call` (default)
Parity intent usage:
`api-rest call [--env <name> | --url <url>] [--token <name>] [--config-dir <dir>] [--no-history] <request.request.json>`

Flags:
- `-e, --env <name>`: select an endpoint preset from `endpoints.env`.
- `-u, --url <url>`: set an explicit base URL (takes precedence over `--env` and `REST_URL`).
- `--token <name>`: select a token profile name.
- `--config-dir <dir>`: seed setup-dir discovery (or pin it when the seed contains no known files).
- `--no-history`: disable writing to `.rest_history` for this run only.
- `-h, --help`: print help and exit `0`.

#### `api-rest history`
Parity intent usage:
`api-rest history [--config-dir <dir>] [--file <path>] [--last | --tail <n>] [--command-only]`

Flags:
- `--config-dir <dir>`: seed setup-dir discovery for the history file.
- `--file <path>`: explicit history file path (relative paths are resolved against the discovered setup dir).
- `--last`: print the last entry (default).
- `--tail <n>`: print the last `n` entries.
- `--command-only`: omit the metadata line (lines starting with `#`) from each entry.
- `-h, --help`: print help and exit `0`.

#### `api-rest report`
Parity intent usage:
`api-rest report --case <name> --request <request.request.json> [options]`

Core options:
- `--case <name>`: required human label for the report.
- `--request <file>`: required request file path.
- `--run`: execute the request via `api-rest call` and embed the response (+ captured stderr).
- `--response <file|->`: embed an existing response (use `-` for stdin); request is not executed.
- `--out <path>`: output report path (default described below).
- `--no-redact`: disable token/password field redaction in JSON request/response sections.
- `--no-command`: omit the command snippet section in the report.
- `--no-command-url`: when `--url` is used, omit the URL value in the command snippet (see report semantics).
- `--project-root <path>`: override the project root used to compute default output path and relative paths.
- `--config-dir <dir>`: passed through to `api-rest call` (setup dir selection).

Endpoint/auth pass-through options:
- `-e, --env <name>`
- `-u, --url <url>`
- `--token <name>`

Mutual exclusivity rules (parity):
- `--run` and `--response` MUST NOT be used together (error).
- At least one of `--run` or `--response` MUST be provided (error).

### Environment variables (by function)

#### Endpoint selection
- `REST_URL=<url>`
  - Used only when neither `--env` nor `--url` is provided.
  - Overridden by `--env` / `--url`.
- `REST_ENV_DEFAULT` (file-based; see “endpoints files”)
  - Read from `endpoints.env` / `endpoints.local.env` (not from the process environment in the legacy scripts).

#### Auth selection
- `ACCESS_TOKEN=<token>`
  - Used only when no token profile is selected.
  - When present, sends `Authorization: Bearer <token>`.
- `SERVICE_TOKEN=<token>`
  - Fallback for `ACCESS_TOKEN` when no token profile is selected.
- `REST_TOKEN_NAME=<name>`
  - Selects a token profile name (equivalent to `--token <name>`).
  - Presence of this variable counts as “token profile selected” (affects `ACCESS_TOKEN` fallback rules).

#### JWT validation controls
These only apply when a token is present (selected profile token or `ACCESS_TOKEN`/`SERVICE_TOKEN` fallback):
- `REST_JWT_VALIDATE_ENABLED=true|false` (default: `true`)
- `REST_JWT_VALIDATE_STRICT=true|false` (default: `false`)
- `REST_JWT_VALIDATE_LEEWAY_SECONDS=<int>` (default: `0`, clamped to `>= 0`)

Boolean parsing parity:
- Values are trimmed and lowercased.
- Only `true` and `false` are accepted.
- Any other value prints a warning to stderr and is treated as `false`.

#### History controls (call)
- `REST_HISTORY_ENABLED=true|false` (default: `true`)
- `REST_HISTORY_FILE=<path>` (default: `<setup_dir>/.rest_history`; relative paths are resolved against the setup dir)
- `REST_HISTORY_LOG_URL_ENABLED=true|false` (default: `true`)
- `REST_HISTORY_MAX_MB=<int>` (default: `10`; `0` disables rotation; clamped to `>= 0`)
- `REST_HISTORY_ROTATE_COUNT=<int>` (default: `5`; clamped to `>= 1`)

#### Report controls
- `REST_REPORT_DIR=<path>`
  - Default directory for generated reports when `--out` is not set.
  - If relative, it is resolved against `<project_root>`.
  - Default: `<project_root>/docs`.
- `REST_REPORT_INCLUDE_COMMAND_ENABLED=true|false` (default: `true`)
  - If `false`, omits the command snippet section (equivalent to `--no-command`).
- `REST_REPORT_COMMAND_LOG_URL_ENABLED=true|false` (default: `true`)
  - If `false`, omits the URL value in the command snippet when `--url` is used (equivalent to `--no-command-url`).

## Setup/config discovery

### Setup dir definition
The “setup dir” is the directory that contains REST config files. Canonical location: `setup/rest`.

The setup dir influences:
- Endpoint preset resolution (`endpoints.env` + `endpoints.local.env`)
- Token profile resolution (`tokens.env` + `tokens.local.env`)
- History file location (`<setup_dir>/.rest_history`)

### File parsing rules (endpoints/tokens env files)
The legacy scripts parse `.env`-like files with these rules:
- Blank lines and lines starting with `#` are ignored.
- Lines may be `KEY=VALUE` or `export KEY=VALUE`.
- Values may be wrapped in single or double quotes (quotes are stripped).
- If a key is assigned multiple times, the last assignment wins.
- Local override files are read after the base file, so the local value wins.

Important parity quirk:
- `endpoints.local.env` is only consulted when `endpoints.env` exists.
  - A repo with only `endpoints.local.env` will not support `--env` selection in the legacy behavior.
- `tokens.local.env` may exist without `tokens.env` and is still used.

### Setup dir discovery: `api-rest call`
Parity algorithm (ported from `rest.sh`):
1. Seed directory:
   - If `--config-dir` is set: seed = `--config-dir`.
   - Else: seed = directory containing the request file.
2. Search upward from the seed directory for the first matching file, in order:
   - `endpoints.env`
   - `tokens.env`
   - `endpoints.local.env`
   - `tokens.local.env`
   If found: `setup_dir` is the directory containing that file.
3. Else, search upward from the seed directory for a `setup/rest` directory.
   If found: `setup_dir` is that `setup/rest` directory.
4. Else, if `--config-dir` was explicitly set: use the seed directory as `setup_dir`.
5. Else, search upward from the *invocation directory* (the process CWD) for `setup/rest`.
   If found: use that.
6. Else: use the seed directory.

### Setup dir discovery: `api-rest history`
Parity algorithm (ported from `rest-history.sh`):
1. Seed directory:
   - If `--config-dir` is set: seed = `--config-dir`.
   - Else: seed = current directory.
2. Search upward from the seed directory for the first matching file, in order:
   - `.rest_history`
   - `endpoints.env`
   - `tokens.env`
   - `tokens.local.env`
   If found: `setup_dir` is the directory containing that file.
3. Else, search upward from the seed directory for a `setup/rest` directory.
   If found: `setup_dir` is that `setup/rest` directory.
4. Else: use the seed directory.

## Endpoint selection rules
Ported from `rest.sh`.

Endpoint resolution order (highest precedence first):
1. `--url <url>`
2. `--env <name>`
   - If `<name>` looks like a URL (`^https?://`), treat it as a URL (equivalent to `--url`).
   - Otherwise, resolve from `endpoints.env` (+ optional `endpoints.local.env`) using:
     - `<ENV_KEY> = uppercased(<name>) with non-alphanumerics replaced by '_'` (trim/collapse underscores)
     - URL = `REST_URL_<ENV_KEY>`
3. `REST_URL=<url>` environment variable
4. `REST_ENV_DEFAULT` (from endpoints files) resolved as `REST_URL_<ENV_KEY>`
5. Default: `http://localhost:6700`

If `--env <name>` is used (and is not a URL) but `endpoints.env` cannot be found under the setup dir:
- Fail with a clear error.

If `--env <name>` is unknown:
- Fail with a clear error listing available env presets discovered from `endpoints.env` (+ `endpoints.local.env` if present).

## Auth selection rules
Ported from `rest.sh`.

### Token profile selection
A “token profile is selected” if *any* of these sources provides a profile name:
- CLI: `--token <name>`
- Environment: `REST_TOKEN_NAME=<name>`
- Tokens files: `REST_TOKEN_NAME=<name>` in `tokens.env` or `tokens.local.env`

If no source provides a profile name, the token profile is considered “not selected” (even though the internal default name becomes `"default"`).

Token profile name normalization:
- The selected name is trimmed and lowercased for display/logging.
- When looking up the token variable, the name is converted to an env key (`<NAME_KEY>`) by uppercasing and converting non-alphanumerics to `_`.
  - Example: `local-dev` → `LOCAL_DEV` → lookup key `REST_TOKEN_LOCAL_DEV`.

### Token source resolution
If a token profile is selected:
- Read the bearer token from `tokens.env` / `tokens.local.env` using `REST_TOKEN_<NAME_KEY>`.
- If the selected token is empty or missing:
  - Fail hard with a helpful message listing available token profile names.
  - Suggest using `ACCESS_TOKEN` *without selecting a token profile*.

If a token profile is NOT selected:
- If `ACCESS_TOKEN` is set: use it as the bearer token.
- Else if `SERVICE_TOKEN` is set: use it as the bearer token.
- Else: no `Authorization` header is sent.

### Authorization header behavior
When a bearer token is selected (from either source):
- Send `Authorization: Bearer <token>`.
- Request files may not set `Authorization` headers (they are ignored for parity).

## JWT validation behavior
Ported from `rest.sh` (which uses Python for the legacy implementation).

Scope:
- This validates *JWT shape and time-based claims only* (no signature verification).
- Validation runs only when a bearer token is present.
- Validation is enabled by default and can be disabled with `REST_JWT_VALIDATE_ENABLED=false`.

Time checks:
- `exp` (if present) must be parseable as a number (integer-ish). If `exp < now - leeway`, the token is treated as expired.
- `nbf` (if present) must be parseable as a number. If `nbf > now + leeway`, the token is treated as not-yet-valid.
- `REST_JWT_VALIDATE_LEEWAY_SECONDS` is an integer number of seconds to allow for clock skew (default `0`).

Format checks:
- Token must be three dot-separated segments.
- Header and payload must be base64url-decodable JSON objects.
- `exp`/`nbf` values, when present, must not be booleans and must be parseable as numeric values.

Failure policy:
- If the token is expired (`exp`) or not-yet-valid (`nbf`): always fail the request (exit non-zero).
- If the token is not a valid JWT (wrong segment count / decode / JSON / invalid claim types):
  - Strict mode (`REST_JWT_VALIDATE_STRICT=true`) fails the request.
  - Non-strict mode prints a warning and proceeds (skipping further format validation).

Legacy missing-runtime behavior (for parity awareness):
- If the legacy script cannot run `python3`, it prints a warning and skips JWT validation entirely.

## Request schema (JSON)
Request files are JSON-only and must parse as a JSON object.

Canonical example (v1):
```json
{
  "method": "GET",
  "path": "/health",
  "query": { "verbose": true, "tags": ["a", "b"] },
  "headers": { "X-Request-Id": "abc123" },
  "body": { "hello": "world" },
  "multipart": [
    { "name": "file", "filePath": "./sample.png", "contentType": "image/png" }
  ],
  "cleanup": {
    "method": "DELETE",
    "pathTemplate": "/files/images/{{key}}",
    "vars": { "key": ".key" },
    "expectStatus": 204
  },
  "expect": { "status": 200, "jq": ".ok == true" }
}
```

Notes:
- `body` and `multipart` are mutually exclusive.
- The runner does not require the `.request.json` suffix; it is a convention only.

### `method` (required)
- Must be a non-empty string.
- Trimmed, uppercased.
- Must match `^[A-Z]+$` (letters only).

### `path` (required)
- Must be a non-empty string.
- Must start with `/`.
- Must not include:
  - `://` (must be a relative path; no scheme/host)
  - `?` (query string is represented by `.query`)

### `query` (optional)
If present and not `null`:
- Must be an object.
- Each value must be:
  - a scalar (`string`, `number`, `boolean`), or
  - an array of scalars (array elements must not be `object` or `array`).
- `null` values are omitted.

Encoding rules:
- Query entries are sorted by key (ascending).
- Each key and value is URI-encoded.
- Arrays are encoded as repeated `key=value` pairs, in the array’s original order.

### `headers` (optional)
If present and not `null`:
- Must be an object.
- Keys must match `^[A-Za-z0-9-]+$`.
- Values must be scalars; `object` and `array` values are rejected.
- `null` values are omitted.

Reserved headers:
- Any header key case-insensitively equal to `Authorization` or `Content-Type` is ignored.
- If there is no user-provided `Accept` header (case-insensitive match), the runner adds:
  - `Accept: application/json`

### `body` (optional)
Presence is keyed on whether the `body` field exists (even if the value is `null`).

If present:
- The runner adds `Content-Type: application/json`.
- The request body is the JSON encoding of the `body` value.
  - Example: if `"body": "hi"`, the bytes sent are the JSON string `"hi"` (including quotes).

### `multipart` (optional)
Presence is keyed on whether the `multipart` field exists.

Rules:
- `multipart` MUST NOT be used together with `body` (error).
- If present and not `null`, it must be an array.
- Each element must be an object (a “part”).

Part object fields:
- `name` (required): non-empty string (trimmed).
- `value` (optional): if present and non-empty after trimming, sends `-F name=value` (stringified).
- `filePath` (optional): local path to a file to upload.
- `base64` (optional): base64 payload to decode into a temporary file for upload.
- `filename` (optional): overrides filename in the multipart upload.
- `contentType` (optional): sets a per-part content type.

Part selection precedence (parity with the legacy script):
1. If `value` is present and non-empty → use it (ignore other fields).
2. Else if `base64` is present and non-empty → decode to a temp file and upload that file.
3. Else require `filePath` to be present and point to an existing file.
4. Otherwise error: part must include `value`, `filePath`, or `base64`.

Upload formatting:
- If `filename` and/or `contentType` are provided, the upload uses the equivalent of:
  - `-F "name=@<path>;filename=<filename>;type=<contentType>"`
- Otherwise:
  - `-F "name=@<path>"`

### `expect` (optional; CI/E2E contract)
Presence is keyed on whether the `expect` field exists.

If present:
- `expect.status` is required and must be an integer (stringified numeric is accepted by legacy parsing).
- `expect.jq` is optional and must be a string (trimmed).

Evaluation rules:
- If `expect` is present, the request is treated as a strict contract:
  - The HTTP status MUST equal `expect.status`.
  - If `expect.jq` is provided, it is evaluated against the response body as JSON.
    - `expect.jq` is only evaluated if the status check passed.
- If `expect` is not present:
  - Any non-2xx status is considered failure.

### `cleanup` (optional)
Presence is keyed on whether the `cleanup` field exists.

When cleanup runs:
- Cleanup is executed only after the main request is considered successful (including `expect` checks when present).
- Cleanup uses the same base URL and bearer token (if any).
- Cleanup response bodies are ignored; only status is checked.

Fields:
- `cleanup.method` (optional): defaults to `DELETE`.
- `cleanup.pathTemplate` (required): a string containing placeholders like `{{key}}`.
- `cleanup.vars` (optional): object mapping placeholder name → jq expression.
- `cleanup.expectStatus` (optional): defaults to `204` when method is `DELETE`, otherwise `200`.

Template substitution rules:
- Each key in `cleanup.vars` is substituted into `cleanup.pathTemplate` by replacing `{{<key>}}` with the jq-evaluated value.
- Each jq expression is evaluated against the *main response body* as JSON.
- The first output line is used (then trimmed).
- Empty / `null` values are errors.
- After substitution, the final cleanup path MUST start with `/` (absolute path).

## Execution & output semantics
Ported from `rest.sh`.

### URL construction
- Base URL is `rest_url` with any trailing slash removed.
- Full URL is `${rest_url%/}${path}`.
- If a query string exists, append `?` + query string.

### Headers set by the runner
- `Accept: application/json` is added unless the user provided an `Accept` header.
- `Content-Type: application/json` is added when `body` is present.
- `Authorization: Bearer <token>` is added when a token is selected (profile or env).
- User headers are appended after these defaults (except reserved headers, which are ignored).

### Stdout and stderr contract
- Stdout: prints the response body bytes exactly as returned (legacy prints it even on failure paths).
- Stderr: prints:
  - input validation failures (missing fields, invalid schema)
  - endpoint/token selection errors
  - HTTP client errors
  - expectation failures (`expect.status`, `expect.jq`)
  - cleanup failures

### Failure-body echo behavior (non-interactive)
On failure, when stdout is *not* a TTY:
- If the response body is valid JSON, no extra body is printed to stderr.
- If the response body is not JSON and non-empty, print:
  - `Response body (non-JSON; first 8192 bytes):`
  - followed by the first 8192 bytes of the body.

## History semantics
Ported from `rest.sh` + `rest-history.sh`.

### When history is written
- History is appended only for `api-rest call` (not for `history` or `report` commands).
- History is enabled by default and may be disabled by:
  - `--no-history` (for that run), or
  - `REST_HISTORY_ENABLED=false`
- History append happens on process exit (including failure exits) when enabled.

### History file location
Default: `<setup_dir>/.rest_history`

Overrides:
- If `REST_HISTORY_FILE` is set:
  - absolute paths are used as-is
  - relative paths are resolved against `<setup_dir>`

### Locking and best-effort writes
The legacy script uses a lock directory (`<history_file>.lock`) to avoid concurrent writes.
If the lock cannot be acquired, history is skipped silently (no error).

### Rotation
If `REST_HISTORY_MAX_MB > 0` and the history file size is `>= REST_HISTORY_MAX_MB * 1024 * 1024`:
- Rotate the file to `.1`, `.2`, …, keeping `REST_HISTORY_ROTATE_COUNT` files.
- Rotation is best-effort; failures do not fail the request.

### Record format
Each entry is a blank-line-separated record:
1. A metadata line starting with `#`.
2. A copy/paste command snippet.
3. A trailing blank line separating records.

Metadata line shape (parity intent):
`# <timestamp> exit=<code> setup_dir=<rel> [env=<name> | url=<url>|url=<omitted>] [token=<name> | auth=ACCESS_TOKEN]`

Details:
- Timestamp format is `YYYY-MM-DDTHH:MM:SS%z` (for example `2026-01-31T12:34:56-0800`) when supported by the platform `date`.
- `setup_dir=<rel>` is the setup dir rendered relative to the invocation directory when possible.
- URL logging is controlled by `REST_HISTORY_LOG_URL_ENABLED`:
  - If `false` and the endpoint is URL-based, metadata uses `url=<omitted>` and the command snippet omits `--url`.
- Tokens are never logged by value:
  - Token profile names may be logged (`token=<name>`).
  - When `ACCESS_TOKEN` is used (and no token profile is selected), metadata uses `auth=ACCESS_TOKEN`.

Command snippet shape (parity intent):
- Multi-line with `\` continuations.
- Includes `--config-dir <setup_dir>` to pin configuration for replay.
- Ends with `| jq .` for human-friendly JSON formatting (even if `api-rest` itself prints raw JSON).

Note on Rust port output:
- The legacy history snippet uses a `$CODEX_HOME/.../rest.sh` path when available.
- The Rust port MAY emit an equivalent `api-rest` invocation instead, but must preserve the record structure and option semantics.

### `api-rest history` output rules
- Entries are parsed as blank-line-separated records.
- `--last` prints the last record.
- `--tail <n>` prints the last `n` records.
- `--command-only` drops the metadata line if it begins with `#`.
- Output includes a blank line after each printed record (including the last).

## Report semantics
Ported from `rest-report.sh` and aligned with `rest-api-testing/references/REST_API_TEST_REPORT_CONTRACT.md`.

### Output path and printing behavior
On success, the command prints the report path to stdout and exits `0`.

Default output path when `--out` is not set:
- `stamp = YYYYMMDD-HHMM` (local time)
- `case_slug = lowercased(case) with non-alphanumerics replaced by '-'` (trim/collapse hyphens; fallback `"case"`)
- `report_dir = REST_REPORT_DIR` if set, else `<project_root>/docs`
  - if `REST_REPORT_DIR` is relative, it is resolved against `<project_root>`
- `out = <report_dir>/<stamp>-<case_slug>-api-test-report.md`

Project root resolution:
- Default: the Git repo root (if available); otherwise the current directory.
- Override with `--project-root`.

### Response sourcing (`--run` vs `--response`)
`--run`:
- Executes `api-rest call` using the provided endpoint/auth/config options.
- Captures:
  - stdout as the response body
  - stderr (if non-empty) into a `### stderr` section
- Records `Result: PASS` when the call exit code is `0`, else `Result: FAIL (api-rest exit=<code>)`.

`--response <file|->`:
- Reads response bytes from the given file (or stdin).
- Does not execute the request.
- Records `Result: (response provided; request not executed)`.

### Report Markdown structure
The report is Markdown and includes (parity intent):
- `# API Test Report (<YYYY-MM-DD>)`
- `## Test Case: <case>`
- Optional `## Command` section with a fenced `bash` block
- `Generated at: <timestamp-with-timezone>`
- Endpoint note:
  - `Endpoint: --url <url>` OR `Endpoint: --env <name>` OR `Endpoint: (implicit; see REST_URL / REST_ENV_DEFAULT)`
- Result note (`PASS`/`FAIL`/provided)
- Optional `### Assertions` section when the request includes `expect`
- `### Request` (fenced `json` block)
- `### Response` (fenced `json` or `text` block)
- Optional `### stderr`

### Redaction rules
Redaction is ON by default (for JSON formatting only).
When enabled, any object field with these keys is replaced with `<REDACTED>` (deep traversal):
- `accessToken`, `refreshToken`, `password`, `token`, `apiKey`
- `authorization`, `Authorization`
- `cookie`, `Cookie`
- `set-cookie`, `Set-Cookie`

Notes:
- Redaction only applies when the request/response is parseable as JSON.
- When the response is non-JSON text, it is included verbatim (no redaction).

### Command snippet inclusion and URL elision
Command snippet inclusion:
- Default: included.
- Omitted when:
  - `--no-command`, or
  - `REST_REPORT_INCLUDE_COMMAND_ENABLED=false`

URL in command snippet (when `--url` is used):
- Default: included.
- When disabled (`--no-command-url` or `REST_REPORT_COMMAND_LOG_URL_ENABLED=false`), the legacy script renders:
  - `--url "<omitted>"`
  (the line is kept but the value is replaced).

### Assertion evaluation in reports
When the request includes `expect`:
- In `--run` mode:
  - Assertions are marked `PASS` if the call exit code is `0`, else `FAIL`.
- In `--response` mode:
  - `expect.status` is `NOT_EVALUATED`.
  - If the response is JSON and `expect.jq` is present, evaluate it and mark `PASS`/`FAIL`.
  - If the response is not JSON, `expect.jq` is `NOT_EVALUATED`.

## Exit codes

### `api-rest call`
- `0`: request executed successfully, and:
  - if `expect` is present: `expect.status` matched and (if present) `expect.jq` evaluated true, and cleanup (if present) succeeded.
  - if `expect` is absent: HTTP status was 2xx, and cleanup (if present) succeeded.
- `1`: invalid input/config, HTTP client failure, non-2xx status without `expect`, expectation failure, cleanup failure, or JWT policy failure.

### `api-rest history`
- `0`: printed at least one record successfully.
- `1`: invalid arguments, setup/history discovery failure, or history file missing.
- `3`: history file exists but contains zero records (parity with the legacy awk-based implementation).

### `api-rest report`
- `0`: report written and the output path printed (even if `--run` produced a failing request; the failure is recorded in the report).
- `1`: invalid arguments, missing files, failure to write report, or JSON formatting failures (legacy requires `jq`).

## External dependencies (inventory + policy)

### Legacy script inventory (observed)
| Tool / runtime | Used by | Purpose | Legacy status |
|---|---|---|---|
| `curl` | `rest.sh` | HTTP execution | required |
| `jq` | `rest.sh`, `rest-report.sh` | JSON parsing/validation, query encoding, `expect.jq`, cleanup vars, formatting/redaction | required |
| `python3` | `rest.sh` | JWT validation; base64 multipart decoding | optional (feature-dependent) |
| `git` | `rest-report.sh` | detect project root | optional (falls back to CWD) |
| `awk` | `rest-history.sh` | record parsing (`RS=`) | required |
| coreutils (`date`, `mktemp`, `wc`, `mv`, `mkdir`, `head`, etc.) | all | plumbing | required |

### Rust port policy (api-rest)
Goal: eliminate runtime dependencies on external binaries for core behavior.

| Dependency | Policy | Rationale |
|---|---|---|
| `curl` | eliminate | implement HTTP client in Rust for portability and consistent error handling |
| `jq` | eliminate (default) | implement JSON formatting + jq-like evaluation (`expect.jq`, cleanup vars, redaction) in Rust |
| `python3` | eliminate | implement JWT parsing/time checks and base64 decoding in Rust |
| `git` | eliminate | detect repo root in Rust (or treat as best-effort “CWD” when not in a repo) |
| shell utilities (`awk`, `date`, `mktemp`, etc.) | eliminate | implement history/report generation and file ops in Rust |

If a compatibility fallback to any external tool is added later, it MUST:
- be explicitly opt-in (flag/env),
- document missing-tool behavior in `--help`,
- and be covered by fixtures/tests.
# api-rest fixtures

These fixtures define deterministic scenarios for integration tests. Tests should use a local HTTP server and temporary
`setup/rest` directories (do not rely on external network).

## call: 2xx success (no expect)
- Setup:
  - Start a local HTTP server that returns `200` with JSON body: `{"ok":true}` for `GET /health`.
  - Create `setup/rest/requests/health.request.json` with `method=GET`, `path=/health`.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/health.request.json`
- Expect:
  - exit `0`
  - stdout is the response body JSON
  - stderr is empty

## call: non-2xx fails when expect is absent
- Setup:
  - Server returns `500` with JSON body (or plain text body).
  - Request file targets that endpoint.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/fail.request.json`
- Expect:
  - exit `1`
  - stdout still contains the response body (parity with `rest.sh`)
  - stderr contains an HTTP failure message

## call: expect.status mismatch
- Setup:
  - Server returns `200` for `GET /health`.
  - Request includes `"expect": { "status": 201 }`.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/health.request.json`
- Expect:
  - exit `1`
  - stderr contains `Expected HTTP status 201 but got 200.`

## call: expect.jq failure
- Setup:
  - Server returns `200` with JSON body: `{"ok":false}`.
  - Request includes `"expect": { "status": 200, "jq": ".ok == true" }`.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/health.request.json`
- Expect:
  - exit `1`
  - stderr contains `expect.jq failed: .ok == true`

## call: strict query encoding
- Setup:
  - Request includes `.query` with:
    - scalar values, arrays of scalars, nulls (omitted), and at least one invalid object value.
- Command: run `api-rest call ...`
- Expect:
  - valid query produces deterministic ordering and encoding
  - invalid query produces a hard-fail with a clear error (objects rejected)

## call: user headers validation and filtering
- Setup:
  - Request includes `.headers` containing:
    - a valid header key/value,
    - an invalid header key (`"Bad Key": "x"`),
    - an `Authorization` header (must be rejected/ignored),
    - a `Content-Type` header (must be rejected/ignored).
- Command: run `api-rest call ...`
- Expect:
  - invalid header key hard-fails with an actionable message
  - forbidden headers are not applied from the request file

## call: multipart file upload (filePath)
- Setup:
  - Create a temp file `setup/rest/requests/fixtures/upload.bin`.
  - Server accepts multipart at `POST /upload` and echoes metadata as JSON.
  - Request includes `multipart` part with `{ "name": "file", "filePath": "setup/rest/requests/fixtures/upload.bin" }`.
- Command: `api-rest call --url http://127.0.0.1:<port> setup/rest/requests/upload.request.json`
- Expect:
  - exit `0`
  - server receives a multipart body with the file

## call: multipart file upload (base64)
- Setup:
  - Request includes `multipart` part with `{ "name": "file", "base64": "<base64 payload>", "filename": "x.bin" }`.
  - Server accepts the upload.
- Command: run `api-rest call ...`
- Expect:
  - exit `0`
  - base64 payload is decoded and uploaded

## call: cleanup success
- Setup:
  - Main request returns JSON body containing a cleanup key, for example `{ "key": "abc" }`.
  - Request includes:
    - `cleanup.pathTemplate="/files/{{key}}"`
    - `cleanup.vars={ "key": ".key" }`
  - Server returns `204` for `DELETE /files/abc`.
- Command: run `api-rest call ...`
- Expect:
  - exit `0`
  - cleanup request is executed after a successful main request

## call: cleanup failure
- Setup:
  - Cleanup endpoint returns a non-expected status.
- Command: run `api-rest call ...`
- Expect:
  - exit `1`
  - stderr contains `cleanup failed: expected ... but got ...`

## history: append and command-only output
- Setup:
  - Run a successful call with `--config-dir setup/rest` so the setup dir is unambiguous.
  - Ensure `.rest_history` is created under that setup dir.
- Command:
  - `api-rest history --config-dir setup/rest --last`
  - `api-rest history --config-dir setup/rest --command-only`
- Expect:
  - history entry starts with a metadata line beginning with `#`
  - command-only output omits the metadata line
  - command snippet includes `--config-dir` and ends with `| jq .`
  - token values are never present; only token profile names may appear

## history: rotation
- Setup:
  - Set `REST_HISTORY_MAX_MB=0` to disable rotation (control case), then set a very small value to force rotation.
  - Run enough calls to exceed the threshold.
- Command: run repeated calls.
- Expect:
  - rotated files exist (for example `.rest_history.1`) and are bounded by `REST_HISTORY_ROTATE_COUNT`.
