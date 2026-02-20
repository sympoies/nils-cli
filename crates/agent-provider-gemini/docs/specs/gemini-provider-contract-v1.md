# Gemini provider contract v1

## Purpose

Define the stable `agent-provider-gemini` behavior for `provider-adapter.v1` and keep compatibility
rules explicit for `agentctl` and downstream consumers.

## Metadata

- `id`: `gemini`
- `contract_version`: `provider-adapter.v1`
- `maturity`: `stable`

## Operations

### capabilities

- Always returns provider-neutral capability objects.
- `execute.available` is true only when:
  - `gemini` binary is present on `PATH`, and
  - `GEMINI_ALLOW_DANGEROUS_ENABLED=true`.
- `include_experimental=true` adds:
  - `diag.rate-limits` (`gemini-cli.diag.rate-limits.v1`)
  - `auth.commands` (`gemini-cli.auth.v1`)

### healthcheck

- `unhealthy` when `gemini` binary is missing.
- `degraded` when binary exists but dangerous policy or auth file readiness is not satisfied.
- `healthy` when binary exists, dangerous policy is enabled, and auth file is present.
- `details` must include binary availability, policy status/message, auth file path/existence, and
  requested timeout.

### execute

- Source prompt from `input` (if non-empty) else trimmed `task`.
- Returns validation error `missing-task` when neither is usable.
- Returns dependency error `missing-binary` when `gemini` is unavailable.
- Returns validation error `disabled-policy` when dangerous mode is disabled.
- Calls `gemini-core::exec::exec_dangerous` when preconditions pass.
- On non-zero exit, returns internal error `execute-failed` with stable details
  (`exit_code`, `stderr`, `task`).

### limits

- Returns deterministic static limits:
  - `max_concurrency=1`
  - `max_timeout_ms=null`
  - `max_input_bytes=null`

### auth-state

- `unknown` when auth file path cannot be resolved.
- `unauthenticated` when auth file path resolves but file is missing.
- Parse auth file via `gemini-core` helpers (`email`, `identity`, `account_id`).
- `authenticated` with `subject` from first available claim in precedence:
  `email -> identity -> account_id`.
- Malformed auth payloads return auth error `invalid-auth-file`.

## Error taxonomy

- `validation`:
  - `missing-task`
  - `disabled-policy`
- `dependency`:
  - `missing-binary`
- `internal`:
  - `execute-failed`
- `auth`:
  - `invalid-auth-file`

All error codes are stable and regression-protected by adapter contract tests.

## Compatibility

- Contract ID/version remains `provider-adapter.v1`.
- Runtime dependency edge remains `agent-provider-gemini -> gemini-core`.
- Provider-to-CLI imports are forbidden (`gemini_cli::*`).
- Changes to error codes/categories, health status mapping, or operation shapes are breaking unless
  accompanied by an explicit compatibility plan and migration notes.
