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

## Shared fetch and cooldown

OAuth usage readers on one host share a process-locked response cache and
cooldown in the prompt-segment cache directory. A successful response is reused
for 60 seconds across prompt segments, usage, diagnostics, and reset-status
reads. The shared request includes the Claude Code status query so every reader
can use the same payload. Reusing it preserves its original fetch time.

The key is a digest of the account UUID from a matching stored profile or the
active login metadata. Tokens without known account metadata use a token digest;
readers must share the same cache directory and account metadata to coalesce.
Neither tokens nor account identifiers are written to the shared state.

A rate limit uses positive `Retry-After` (seconds or an HTTP date), or a
300-second default. Consecutive local delays double up to one hour; a longer
server hint is never shortened. Success resets the delay. Existing token-keyed
cooldowns migrate to the account key. Cache clearing does not clear cooldowns
or bypass the shared fetch interval. Reset submissions invalidate the shared
response because they can change usage and grant availability.

All readers can display the same eligible last snapshot while throttled, with
its original age and the existing ten-minute display ceiling. `usage --source
auto` skips the native `/usage` probe after an OAuth rate limit. Diagnostic tables
show the throttle deadline and cache age, including the age of a snapshot too
old to display. With no recorded default, the table highlights the stored
profile matching the active login's token.

## Follow-up: cross-host authority

Each host still fetches independently. A configured authority could publish
account-scoped usage snapshots and retry deadlines for replicas to read, reducing
requests across hosts. This needs an explicit transport, freshness and authority
failure contract; it is deferred from the host-local coordination in Refs #3039.
