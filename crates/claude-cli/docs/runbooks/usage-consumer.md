# Claude Usage Consumer Runbook

## Consumer contract

`agent-session` invokes `claude-cli usage --format json --source auto` and
consumes `claude-cli.usage.v1`.

Consumers must:

1. validate `schema_version`, `command`, and `ok`;
2. read `result.windows` independently;
3. branch on `result.reason_code`, never `note` or provider prose;
4. treat `stale: true` as non-authoritative for state-changing decisions;
5. accept unknown additive fields.

An empty `windows` array means quota windows could not be established. It must
not be interpreted as unlimited quota or zero usage.

## Failure handling

| Signal | Consumer action |
| --- | --- |
| `auth_required`, `auth_expired` | Ask for supported Claude Code login |
| `billing_past_due`, `subscription_inactive` | Stop retry; surface account action |
| `organization_disabled`, `permission_denied` | Stop retry; surface administrator action |
| `rate_limited` | Display eligible cached windows as stale; let the shared usage cooldown control retry |
| `service_unavailable`, `timeout` | Bounded backoff, then non-authoritative display |
| `unknown` | Preserve diagnostics without parsing text |

Do not parse `note`, stderr, Claude terminal output, or provider response
bodies. Do not inspect Claude credential storage as a fallback.

## Shared cooldown

All OAuth usage callers on one host share the token-keyed persistent cooldown
in the prompt-segment cache directory. A rate limit uses positive `Retry-After`
or a 300-second default, doubles consecutive delays up to one hour, and clears
on a successful HTTP read. `usage --source auto` skips the native `/usage`
probe after an OAuth rate limit. Cache clearing does not clear the cooldown.
