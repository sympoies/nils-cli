# Claude provider contract v1

## Scope

This document specifies `agent-provider-claude` behavior under `provider-adapter.v1`.

## Metadata

- `id`: `claude`
- `contract_version`: `provider-adapter.v1`
- `maturity`: `stable`

## Operations

### `capabilities`

- Always advertises:
  - `capabilities`
  - `healthcheck`
  - `limits`
  - `auth-state`
- `execute.available`:
  - `true` when `ANTHROPIC_API_KEY` is configured and config parsing succeeds.
  - `false` otherwise, with actionable description.
- Experimental capability flags (`include_experimental=true`):
  - `api.messages`
  - `characterization.local-cli`

### `healthcheck`

Deterministic status mapping:

- `healthy`: Claude config is valid and execute path is enabled.
- `degraded`: missing API key (`ANTHROPIC_API_KEY` not set).
- `unhealthy`: invalid config (parse errors).

Response details include:

- `maturity`
- `execute_available`
- `api_key_configured`
- `base_url`
- `model`
- `api_version`
- `timeout_ms`
- `claude_cli_available`
- `requested_timeout_ms`

### `execute`

Input normalization:

- effective prompt is `input` when non-empty, else `task`.
- supports `prompt:`, `advice:`, and `knowledge:` prefixes.

Transport:

- endpoint: `POST {ANTHROPIC_BASE_URL|https://api.anthropic.com}/v1/messages`
- headers: `x-api-key`, `anthropic-version`, `content-type`, `user-agent`
- body: `{model,max_tokens,messages:[{role:user,content:...}]}`

Success:

- `exit_code=0`
- `stdout` = extracted assistant text blocks
- `stderr` may include request metadata (`request_id=<id>`)

### `limits`

- `max_concurrency`: `CLAUDE_MAX_CONCURRENCY` or default `2`
- `max_timeout_ms`: resolved adapter timeout
- `max_input_bytes`: currently unspecified (`null`)

### `auth-state`

- `unauthenticated` when `ANTHROPIC_API_KEY` missing.
- `authenticated` when present.
- `subject` from `ANTHROPIC_AUTH_SUBJECT` or masked key fingerprint.
- `scopes` from `ANTHROPIC_AUTH_SCOPES` (comma-separated).

## Error taxonomy

| Condition | Category | Code | Retryable |
| --- | --- | --- | --- |
| Missing API key | `auth` | `missing-api-key` | `false` |
| HTTP 401/403 | `auth` | `auth-failed` | `false` |
| HTTP 429 | `rate-limit` | `rate-limited` | `true` |
| HTTP 408/504 or request timeout | `timeout` | `request-timeout` | `true` |
| network/connect errors | `network` | `network-error` / `request-failed` | `true` |
| invalid request payloads | `validation` | `invalid-request` / `missing-task` | `false` |
| HTTP 5xx | `unavailable` | `upstream-unavailable` | `true` |
| invalid success JSON from API | `internal` | `invalid-json-response` | `false` |

## Redaction and details policy

- Never include raw API keys in error details or fixtures.
- Include only non-sensitive diagnostics:
  - status code
  - provider error type
  - request id
  - sanitized body snippet/object

## Compatibility rules

- Additive fields in response details are allowed.
- Existing category/code semantics are stable within v1.
- Breaking changes to envelope shape or error semantics require a new contract version.
