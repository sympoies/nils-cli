# Claude provider contract v1

## Scope

This document specifies `agent-provider-claude` behavior under `provider-adapter.v1`.
It is the Claude provider contract for core runtime behavior, not a CLI UX document.

## Ownership boundary (core vs CLI)

Core ownership in `agent-provider-claude`:

- provider contract shape and semantics for `capabilities`, `healthcheck`, `execute`, `limits`, and `auth-state`
- error mapping from Claude/runtime failures into stable `category`, `code`, and `retryable` values
- execution policy (input normalization, transport defaults, timeout/readiness behavior, redaction constraints)

CLI ownership outside provider core modules:

- provider command UX in `agentctl` (flags, prompts, command wording, user-facing command flow)
- `diag` output rendering and presentation details
- `workflow` reporting/rendering in CLI run summaries

Anti-goals:

- anti-goal: do not move CLI rendering concerns (tables, color/styling, human-formatted status text) into provider core modules
- anti-goal: provider core returns structured outcomes; CLI layers own display/routing/report formatting

Boundary regression guardrails:

- contract tests fail if provider core source introduces CLI coupling markers such as `clap::` or `nils-agentctl`
- contract tests fail if provider crate manifest introduces CLI dependencies for provider runtime behavior

## Metadata

- `id`: `claude`
- `contract_version`: `provider-adapter.v1`
- `maturity`: `stable`

## `provider-adapter.v1` schema compatibility

Envelope compatibility expectations:

- `contract_version` is always `provider-adapter.v1`.
- `provider.id` is always `claude`.
- `operation` wire values are `capabilities`, `healthcheck`, `execute`, `limits`, `auth-state`.
- success envelope shape is `status=ok` with `result`.
- error envelope shape is `status=error` with `error` object fields:
  - `category`
  - `code`
  - `message`
  - optional `retryable`
  - optional `details`

Compatibility policy:

- additive fields in `details` are allowed
- existing `category` + `code` semantics are stable within v1
- breaking changes to envelope shape, operation names, or error semantics require a new contract version

## Operations

### `capabilities`

Request:

- accepts `CapabilitiesRequest` with optional `include_experimental`.

Deterministic result behavior:

- always advertises baseline operations:
  - `capabilities`
  - `healthcheck`
  - `limits`
  - `auth-state`
- advertises `execute` with:
  - `available=true` only when `ANTHROPIC_API_KEY` is configured and config parsing succeeds
  - `available=false` otherwise, with actionable description
- with `include_experimental=true`, includes:
  - `api.messages`
  - `characterization.local-cli`

Error behavior:

- returns `ok` (no provider error path for this operation).

### `healthcheck`

Request:

- accepts `HealthcheckRequest.timeout_ms` as a diagnostic hint.

Deterministic result behavior:

- `status=healthy` when Claude config is valid and execute is enabled
- `status=degraded` when API key is missing
- `status=unhealthy` when config parsing fails for reasons other than missing API key
- details include stable diagnostic keys:
  - `maturity`
  - `execute_available`
  - `api_key_configured`
  - `claude_cli_available`
  - `requested_timeout_ms`
- when config is valid, details also include:
  - `base_url`
  - `model`
  - `api_version`
  - `timeout_ms`
- when config is invalid/degraded, details include:
  - `config_error_code`
  - `config_error_message`

Error behavior:

- returns `ok` (health degradation is encoded in response status/details, not provider error envelope).

### `execute`

Request and normalization behavior:

- effective prompt uses `input` when non-empty, otherwise `task`
- supports `prompt:`, `advice:`, and `knowledge:` prefixes through prompt template rendering
- empty normalized prompt returns validation error (`missing-task`)

Transport behavior:

- endpoint: `POST {ANTHROPIC_BASE_URL|https://api.anthropic.com}/v1/messages`
- headers: `x-api-key`, `anthropic-version`, `content-type`, `user-agent`
- body: `{model,max_tokens,messages:[{role:user,content:...}]}`
- retry loop is bounded by `CLAUDE_RETRY_MAX`; only retryable failures are retried

Success behavior:

- `exit_code=0`
- `stdout` is extracted assistant text blocks (fallback: serialized JSON when text blocks absent)
- `stderr` may include `request_id=<id>`
- `duration_ms` is set when measurable

Error behavior:

- emits normalized provider errors using taxonomy in this contract
- `retryable` is explicit for transport/API/client mapping errors
- for local validation (`missing-task`) retryability defaults from category semantics (`validation => false`)

### `limits`

Request:

- accepts `LimitsRequest` (empty payload).

Deterministic result behavior:

- `max_concurrency`: `CLAUDE_MAX_CONCURRENCY` or fallback `2` if unset/invalid
- `max_timeout_ms`: resolved adapter timeout when config is parseable, else `null`
- `max_input_bytes`: `null` (currently unspecified)

Error behavior:

- returns `ok` (invalid limit env values degrade to defaults instead of erroring).

### `auth-state`

Request:

- accepts `AuthStateRequest` (empty payload).

Deterministic result behavior:

- `state=unauthenticated` when `ANTHROPIC_API_KEY` is missing
- `state=authenticated` when key is present
- `subject` is `ANTHROPIC_AUTH_SUBJECT` or masked API key fingerprint
- `scopes` from `ANTHROPIC_AUTH_SCOPES` (comma-separated)
- `expires_at` is currently `null`

Schema alignment notes:

- contract currently emits only `authenticated` and `unauthenticated`; `expired`/`unknown` remain schema-valid reserved states.

Error behavior:

- returns `ok` (authentication readiness is represented in payload state, not provider error envelope).

## Error taxonomy

All emitted error codes are operation-scoped to `execute` in the current stable implementation.

| Operation | Condition | Category | Code | Retryable |
| --- | --- | --- | --- | --- |
| `execute` | Empty normalized task/input | `validation` | `missing-task` | `false` |
| `execute` | Missing `ANTHROPIC_API_KEY` | `auth` | `missing-api-key` | `false` |
| `execute` | Invalid `CLAUDE_*`/`ANTHROPIC_*` config value | `validation` | `invalid-config` | `false` |
| `execute` | HTTP client initialization failure | `internal` | `client-init-failed` | `false` |
| `execute` | HTTP 401/403 | `auth` | `auth-failed` | `false` |
| `execute` | HTTP 429 | `rate-limit` | `rate-limited` | `true` |
| `execute` | HTTP 408/504 or transport timeout | `timeout` | `request-timeout` | `true` |
| `execute` | Transport connect failure | `network` | `network-error` | `true` |
| `execute` | Other transport failure | `network` | `request-failed` | `true` |
| `execute` | HTTP 400/404/422 | `validation` | `invalid-request` | `false` |
| `execute` | HTTP 5xx | `unavailable` | `upstream-unavailable` | `true` |
| `execute` | Success response cannot parse as JSON | `internal` | `invalid-json-response` | `false` |
| `execute` | Non-classified non-5xx HTTP response | `unknown` | `api-error` | `false` |

Category compatibility notes:

- Claude currently emits the subset: `auth`, `rate-limit`, `network`, `timeout`, `validation`, `unavailable`, `internal`, `unknown`.
- `dependency` is valid in schema but not currently emitted by Claude.

## Redaction and details policy

- never include raw API keys in error details, fixtures, or logs
- include only non-sensitive diagnostics:
  - status code
  - provider error type
  - request id
  - sanitized body object/string
  - endpoint and transport error text

## Unsupported behavior handling

- unsupported codex-only UX surfaces are handled by CLI/migration docs, not by adding ad-hoc provider operations.
- provider contract remains limited to `provider-adapter.v1` operations and stable error taxonomy above.
