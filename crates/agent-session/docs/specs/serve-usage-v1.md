# Serve provider usage and resets v1

Part of [Serve API v1](serve-api-v1.md). Its scope and shared rules also apply here.

## Provider operations

### Provider usage and resets

`GET /usage/v1`, `POST /codex/reset/v1`, and `POST /claude/reset/v1` serve
provider usage, the earned Codex rate-limit reset, and the Claude limit
resets from the `codex-cli` and `claude-cli` provider CLIs on `PATH`, so a console edge needs no separate host helper. All three require the
server bearer, like every other authenticated route. Their response shapes
match what a console edge already parses from the helpers they replace; the
synthetic fixtures under `tests/fixtures/usage-v1/` pin that projection.

**Usage.** `GET /usage/v1` returns the ordinary serve envelope with
`data.machine` and `data.usage`:

```json
{
  "schema_version": "agent-session.provider-usage.v1",
  "providers": [
    {
      "provider": "codex",
      "account": "alpha",
      "label": "Codex",
      "ok": true,
      "stale": false,
      "plan": "team",
      "windows": [
        { "key": "5h", "label": "5h", "used_percent": 6, "window_minutes": 300, "resets_at": 1790003600 }
      ],
      "updated_at": 1790000000,
      "note": null,
      "error": null,
      "reason_code": null,
      "reset_credits": { "available_count": 2 }
    }
  ]
}
```

- Codex contributes one entry per account from `codex-cli diag rate-limits --all --format json
  --no-refresh-auth`, in the helper's order. `account` is the profile nickname. `plan` retains a
  non-empty token of at most 64 ASCII letters, digits, dots, underscores, or hyphens, preferring
  `raw_usage.plan_type` and otherwise using the matching account's public broker plan when available.
  This optional enrichment waits at most one second; a broker failure or timeout leaves completed usage
  available with missing plans unknown. `reset_credits` is only a non-negative integer count; absence
  means unknown, not zero.

  Window labels must be `Weekly` or a provider duration such as `5h`, from which `key` and
  `window_minutes` are derived. An account the helper reports as failed is `ok: false` with a fixed
  `error` text and its `reason_code` (`unknown` when unclassified). Account results are preserved even if
  the aggregate helper `ok` is false or its exit status is nonzero; each failure affects only that
  account. A valid all-failed account list also stays per-account. That run also refreshes the shared
  rate-limit cache that `codex-cli account select` reads, so an account selection within the cache TTL
  does no provider fetch.
- Claude contributes one entry per valid profile nickname from `claude-cli diag rate-limits --async
  --format json --jobs 16`. The bounded parallel run disables cache fallback on profile failures so an
  old healthy cache cannot mask the failed account's reason. A successful network result is a fresh `ok:
  true` entry with that nickname in `account`. The active Claude Code login is also included when its
  access token is not already represented by a saved profile. It uses `account: "active"` unless that
  nickname is already used by a profile, in which case it uses an unused `active-login` nickname; an
  optional helper-supplied plan is retained only when it is a bounded token.

  A profile-level failure is an `ok: false`, `stale: true` entry with the account nickname, a fixed
  `error` and `note`, and its classified `reason_code`; an unclassified profile failure uses `unknown`. A
  successful result without windows remains `ok: true` and stale with a fixed `note`. A successful
  no-window result may use the helper's cached fallback; these cached results are stale and their windows
  are hidden because the helper does not expose the cache timestamp.
- A helper can fail to run, time out after 30 seconds, or return an unusable
  document. If the provider has no earlier success, or its last success is
  600 seconds old or more, it is then reported as one entry with
  `account: null`, `stale: true`, and a `reason_code` of `service_unavailable`,
  `timeout`, or the helper's own classified reason. That entry replaces any
  per-account entries because the helper did not provide a usable account
  inventory. Both providers mark it `ok: false` and include a fixed `error`.
- `reason_code` is always `null` or one of `auth_required`, `auth_expired`,
  `billing_past_due`, `subscription_inactive`, `organization_disabled`,
  `permission_denied`, `rate_limited`, `service_unavailable`, `timeout`, or
  `unknown`. `note` and `error` are fixed daemon-owned strings; helper
  messages never pass through. Output never carries credentials, provider
  account ids, emails, file paths, or raw provider responses.

**Caching.** Each provider has a stale-while-refresh cache. A completed snapshot is fresh for 60 seconds
(`AGENT_SESSION_USAGE_V1_REFRESH_SECONDS`, 1 to 300). The first read after that starts one background
refresh and immediately gets the last completed snapshot, with every `ok` entry marked `stale: true` and
a refreshing note. At most one refresh per provider runs at a time. A failed refresh keeps serving that
snapshot with a backoff note and retries after one more interval. Windows are hidden (and the entry
marked stale) once their `updated_at` is 600 seconds old or more than 5 seconds in the future, so a
persistent outage never pins old numbers.

`?refresh=1` marks the current snapshot stale and waits up to 7 seconds for a refresh that starts after
the request. If a refresh is already running, one more run starts when it finishes, so a forced read
never returns numbers from before the request. A cold daemon waits the same bound for its first result.
Any other query is `400 invalid-query`.

**Codex reset.** `POST /codex/reset/v1` takes exactly
`{"account": "<nickname>", "idempotency_key": "<uuid>"}` and consumes at most
one earned reset through
`codex-cli account reset-rate-limits --yes --idempotency-key <uuid> --format json <nickname>.json`.

- `AGENT_SESSION_CODEX_RESET_ACCOUNTS` is the allowlist of nicknames,
  separated by spaces or commas. Unset or empty disables the route with
  `503 codex-reset-not-configured`; an unlisted account is
  `403 codex-reset-account-not-allowed`.
- The idempotency key must be a canonical lowercase UUID. The provider
  receives it as the redemption id, so a repeated key never consumes a second
  credit. The daemon serializes resets and replays a recorded outcome for the
  same key and account for 24 hours (`replayed: true`, no second CLI run); the
  same key for another account is `409 idempotency-key-reused`. A reset keeps
  running and is recorded even when its caller disconnects, so a retry with
  the same key replays it. Recording a reset also marks the Codex snapshot
  stale and starts a refresh, so neither a replay nor a later read serves
  pre-reset numbers as fresh. A failed run is not recorded, so the caller retries
  with the same key.
- A malformed body is `422 invalid-request`. CLI failures are
  `502 codex-reset-failed`, `502 codex-reset-invalid-response`,
  `502 codex-reset-unavailable`, or `504 codex-reset-timeout` (30 seconds, an
  unknown result to retry with the same key). Errors use the ordinary serve
  error envelope and carry no helper output.

Success is not wrapped in the serve envelope. Like the federated mailbox
routes, it returns a raw versioned document whose `schema_version` is the one
the console reset client checks:

```json
{
  "schema_version": "agent-console.codex-rate-limit-reset.v1",
  "outcome": "reset",
  "windows_reset": 2,
  "replayed": false,
  "machine": "workstation",
  "usage": { "schema_version": "agent-session.provider-usage.v1", "providers": [] }
}
```

`outcome` is `reset`, `nothing_to_reset`, `no_credit`, or `already_redeemed`,
and `windows_reset` is present only when the CLI reports it. `usage` is the
`GET /usage/v1` snapshot after a forced Codex refresh. The response waits for
that refresh only until 6 seconds after the request arrived, so a client with
an 8-second timeout still receives the outcome. A slower refresh leaves the
Codex entries `stale: true` with the refreshing note. A replay returns the
cached snapshot instead.

**Claude reset.** `POST /claude/reset/v1` takes exactly
`{"account": "<nickname>", "program": "juniper_tide" | "cedar_ember", "idempotency_key": "<uuid>"}`
and redeems at most one Claude limit reset through
`claude-cli auth reset-rate-limits --yes --program <program> --request-id <uuid> --format json <nickname>`.
`juniper_tide` is the weekly reset of the 5-hour session limit and
`cedar_ember` the next granted reset; `claude-cli` picks the grant on the
host, so grant ids never cross this boundary.

- `AGENT_SESSION_CLAUDE_RESET_ACCOUNTS` is the allowlist of stored
  `claude-cli` profile nicknames, separated by spaces or commas. Unset or
  empty disables the route with `503 claude-reset-not-configured`; an unlisted
  account is `403 claude-reset-account-not-allowed`.
- Replay works as for Codex, in a separate 24-hour record: the same key for
  the same account and program replays the recorded outcome
  (`replayed: true`, no second CLI run), and the same key for another account
  or program is `409 idempotency-key-reused`. The key is also the
  `cedar_ember` request id the provider receives. A reset keeps running and
  is recorded when its caller disconnects; a failed run is not recorded.
  Recording a reset marks the Claude usage snapshot stale and starts a
  refresh, without waiting for it.
- A malformed body is `422 invalid-request`. CLI failures are `502 claude-reset-failed`, `502
  claude-reset-invalid-response`, `502 claude-reset-unavailable`, or `504 claude-reset-timeout` (35
  seconds, an unknown result to retry with the same key). serve runs the CLI with
  `CLAUDE_PROMPT_SEGMENT_MAX_TIME_SECONDS=5` and `CLAUDE_RATE_LIMITS_RESET_MAX_TIME_SECONDS=25`,
  overriding its own environment, so the CLI finishes its status read and POST before that deadline.
  `claude-reset-failed` adds `error.details` when the CLI reported one of its documented error codes:
  `{"cli_code": "<code>", "reason_code": "<reason>" | null, "retryable": <bool>}`.

  `cli_code` is one of `claude-auth-required`, `provider-unavailable`, `provider-rejected`,
  `invalid-provider-response`, `profile-not-found`, `profile-invalid`, `organization-unknown`,
  `endpoint-invalid`, `invalid-profile-name`, `confirmation-required`, `request-id-required`, or
  `invalid-request-id`; `reason_code` uses the usage reason vocabulary; and `retryable` is true only for
  `provider-unavailable`. Helper messages never pass through.

Success is a raw versioned document:

```json
{
  "schema_version": "agent-console.claude-limit-reset.v1",
  "program": "cedar_ember",
  "outcome": "reset",
  "posted": true,
  "reason": null,
  "resets_left": 1,
  "next_available_at": null,
  "cooldown_until": null,
  "weekly_resets_at": 1791334800,
  "replayed": false,
  "machine": "workstation"
}
```

`outcome` is `reset`, `already_used`, `not_limited`, `cooldown`,
`ineligible`, or `unavailable`. `posted` is `false` when `claude-cli` found
the program unavailable and sent nothing. `reason` is `null` or a bounded
lowercase token, `resets_left` a non-negative integer or `null`, and the
three timestamps epoch seconds or `null`. Every field is always present. The
daemon accepts only a CLI result whose envelope, program, outcome, and field
types match; anything else is `claude-reset-invalid-response`.
