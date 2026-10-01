# claude-cli Auth Reset Rate Limits JSON Contract v1

## Purpose

This document defines the explicit mutation contract for
`claude-cli auth reset-rate-limits`. The command redeems at most one Claude
limit reset for exactly one stored profile. Claude has two reset programs,
both evaluated by the Claude OAuth usage endpoint for a Claude Code client:

| Program | Meaning |
| --- | --- |
| `juniper_tide` | One reset of the 5-hour session limit per week, offered only at that limit. Usage still counts toward the weekly limit. |
| `cedar_ember` | Granted resets, each with its own id and expiry. |

The programs are undocumented upstream. This contract follows the shapes
Claude Code itself parses and treats any unexpected status as unavailable.

## Invocation

```text
claude-cli auth reset-rate-limits --program <juniper_tide|cedar_ember>
                                  [--request-id <uuid>] [-y|--yes]
                                  [--format text|json] <profile>
```

- `<profile>` is a stored profile name under `CLAUDE_SECRET_DIR`
  (`<profile>.json`), validated like every other `auth` profile name.
- Non-interactive and JSON invocations require `--yes` and
  `--request-id <uuid>`. The request id must be a canonical lowercase UUID; a
  caller retains it and reuses it when retrying the same logical action after
  an unknown result.
- An interactive invocation without `--yes` reads the status first, reports an
  unavailable program without prompting, and otherwise asks for confirmation
  with a default of no. It generates a request id when none is supplied.

## Provider flow

1. Read the stored profile's `claudeAiOauth.accessToken`, `expiresAt`, and
   `oauthAccount.organizationUuid`. The token is never refreshed or rewritten.
   An expired token is `claude-auth-required` before any request; a profile
   without an organization is `organization-unknown`.
2. Read the status once:
   `GET <usage endpoint>?at_wall=1&skip_spend=1` with the stored token,
   `anthropic-beta: oauth-2025-04-20`, and
   `User-Agent: claude-cli/<Claude Code version> (external, cli)`, the same
   request `diag rate-limits` sends.
3. If the program is not available, return outcome `unavailable` with
   `posted: false` and send nothing else. `juniper_tide` is available when it
   is eligible, available, and not in the `control` arm. `cedar_ember` is
   available when it is eligible and `next_grant_id` names a kept grant with
   `resets_left > 0` that is not paused. A missing or malformed program is
   unavailable with `reason: null`.
4. Otherwise send exactly one
   `POST <api base>/api/organizations/<organization>/reset_rate_limits` with
   `Content-Type: application/json`, the same authorization, beta, and
   User-Agent headers, and exactly one of these bodies:

   ```json
   {"program":"juniper_tide"}
   ```

   ```json
   {"program":"cedar_ember","grant_id":"<next_grant_id>","request_id":"<request id>"}
   ```

   The grant id comes from the status read in step 2, so it never crosses the
   host boundary. The POST is never retried.

The API base is `CLAUDE_RATE_LIMITS_API_BASE_URL`, else the origin of the
usage endpoint (`CLAUDE_PROMPT_SEGMENT_ENDPOINT`, default
`https://api.anthropic.com`). The POST times out after
`CLAUDE_RATE_LIMITS_RESET_MAX_TIME_SECONDS` (default 25, at most 120).

## Envelope

The stable schema id is `claude-cli.auth.reset-rate-limits.v1` and the stable
command is `auth reset-rate-limits`. Every outcome exits `0`:

```json
{
  "schema_version": "claude-cli.auth.reset-rate-limits.v1",
  "command": "auth reset-rate-limits",
  "ok": true,
  "result": {
    "provider": "claude",
    "program": "cedar_ember",
    "outcome": "reset",
    "posted": true,
    "reason": null,
    "resets_left": 1,
    "next_available_at": null,
    "cooldown_until": null,
    "weekly_resets_at": 1791334800
  }
}
```

- `outcome` is `reset`, `already_used`, `not_limited`, `cooldown`,
  `ineligible`, or `unavailable`. Any other upstream `result` is
  `invalid-provider-response`.
- `posted` is `false` only for an `unavailable` outcome decided from the
  status read.
- `reason` is `null` or a bounded token (`^[a-z0-9_]{1,40}$`); any other
  upstream string becomes `unknown`. For `posted: false` it is the status
  `ineligible_reason`.
- `resets_left` is `null` or a non-negative integer.
- `next_available_at`, `cooldown_until`, and `weekly_resets_at` are `null` or
  epoch seconds converted from the upstream ISO-8601 timestamps.

Every field is always present. Text output prints one outcome line.

## Errors

Failures emit the shared JSON error envelope with `ok: false`, no `result`,
and `error.details` holding `retryable`, `reason_code`, and `next_action`.
Upstream response bodies are never forwarded.

| Code | Exit | Meaning |
| --- | --- | --- |
| `invalid-profile-name`, `confirmation-required`, `request-id-required`, `invalid-request-id`, `request-id-unavailable` | `64` | Rejected before reading the profile or sending a request (`request-id-unavailable`: an interactive run could not generate one). |
| `profile-not-found`, `profile-invalid`, `organization-unknown`, `endpoint-invalid` | `1` | The profile or configuration cannot be used. |
| `claude-auth-required` | `2` | Expired token (`reason_code: auth_expired`, no request), or HTTP `401` / `403` (`auth_expired` / `permission_denied`). |
| `provider-unavailable` | `3` | HTTP `429` (`rate_limited`), `5xx` (`service_unavailable`), transport failure, or timeout (`timeout`). `retryable: true`: retry with the same request id. |
| `provider-rejected` | `3` | Any other non-2xx status (not 401, 403, 429, or 5xx), whatever its body says. |
| `invalid-provider-response` | `3` | Unreadable status or reset body, or an unknown result. |

## Privacy boundary

Output never contains access or refresh tokens, authorization headers,
account or organization uuids, grant ids, the request id, the profile file
path, or raw provider response bodies.
