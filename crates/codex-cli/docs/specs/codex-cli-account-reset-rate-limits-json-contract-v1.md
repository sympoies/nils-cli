# codex-cli Account Reset Rate Limits JSON Contract v1

## Purpose

This document defines the explicit mutation contract for
`codex-cli account reset-rate-limits`. The command consumes at most one earned
Codex reset credit for exactly one ChatGPT-authenticated account.

## Invocation safety

- Non-interactive and JSON invocations require `--yes` and
  `--idempotency-key <uuid>`.
- The idempotency key must be a canonical lowercase UUID. A caller must retain
  and reuse the same key when retrying the same logical action after an unknown
  result.
- Interactive invocation first reads and displays the selected-account label
  and available credit count, then prompts with a default of no. It generates a UUID only
  after explicit confirmation when the caller did not supply one.
- The provider request is one POST to
  `/wham/rate-limit-reset-credits/consume` with exactly
  `{"redeem_request_id":"<uuid>"}`. A single authentication retry reuses the
  identical body and key.

## Envelope

The stable schema id is `codex-cli.account.reset-rate-limits.v1` and the stable
command is `account reset-rate-limits`.

Success emits:

```json
{
  "schema_version": "codex-cli.account.reset-rate-limits.v1",
  "command": "account reset-rate-limits",
  "ok": true,
  "result": {
    "provider": "codex",
    "outcome": "reset",
    "windows_reset": 2
  }
}
```

Stable `result.outcome` values are:

- `reset`
- `nothing_to_reset`
- `no_credit`
- `already_redeemed`

`windows_reset` is optional and, when present, is a non-negative integer.

Failure emits the shared top-level JSON error envelope with `ok=false` and no
`result`. Stable local safety codes include `confirmation-required` and
`invalid-idempotency-key`. Provider failures carry a bounded reason code and
retry guidance; raw provider response bodies are not forwarded.

## Privacy boundary

Output must not contain access or refresh tokens, authorization headers,
provider account or credit IDs, the selected account name or filename, the
idempotency key, absolute secret paths, or raw provider response bodies.
