# Mail Audit v1

## Scope and authority

`agent-session message audit` is a read-only owner metadata projection for one
configured machine and state root. It lists overdue unread mail, unresolved
mail targeting stopped/missing sessions or old incarnations, source federation
queue failures, and failed/uncertain notification receipts. It includes all
owners present in that root; session capabilities do not authorize this query.
The local CLI relies on owner filesystem access, like the session board.

`GET /coordination/messages/audit/v1` exposes the same projection under the
existing daemon operator bearer authentication. A session capability alone is
rejected. The endpoint does not grant remote aggregation or collaborator
access; an upstream operator service must separately authorize its callers.

Bodies, body sizes/hashes, tokens, capability digests, idempotency keys/outcomes,
paths, session titles, forwarding content and transcripts are absent. Reduced
read types skip bodies and secret fields when parsing the private stores.
Reason and state fields are allowlisted, with unknown reasons replaced by
`unknown` (notification reasons use `notification-state-invalid`). Queries do
not mark read/acknowledged, normalize receipts, renew brokers/claims, compact,
clean expired records, or persist cursors. They never contact remote hosts.
Resolve/transfer actions and aggregate collectors are outside this contract.

## Query

```sh
agent-session message audit --older-than 300 --limit 50 --format json
agent-session message audit --include-healthy --format json
```

| CLI flag / HTTP parameter | Default | Meaning |
| --- | --- | --- |
| `--older-than` / `older_than` | `300` | Integer seconds; age must be strictly greater. Zero is supported. |
| `--limit` / `limit` | `50` | Records per page, `1..=100`. |
| `--cursor` / `cursor` | absent | Resume with the same machine, threshold and healthy filter. |
| `--include-healthy` / `include_healthy` | `false` | Include all retained metadata, including successful source delivery receipts and acknowledged/expired inbox rows, for joining observations. |

CLI output has the normal `cli.agent-session.message-audit.v1` success/error
envelope. The HTTP route uses the daemon's normal envelope. Both put the
`agent-session.mail-audit.v1` projection under `data`. Parsed range errors and invalid cursors return
`mail-audit-query-invalid` (CLI usage error / HTTP 400). CLI syntax/type errors
retain the standard `parse-error` code; malformed HTTP query values use
`mail-audit-query-invalid`. The
HTTP route rejects unknown query fields. Operator auth failures follow the
existing daemon 401/503 behavior.

Machine identity follows existing configuration (`AGENT_SESSION_MACHINE` and
local identity fallback for the CLI; `serve --machine` for the daemon). Run the
CLI with the same configured identity as the daemon when combining observations.
One root is one observation; collectors own root coverage and offline reporting.

## Projection schema

All fields below are present. Unknown or inapplicable scalar values are JSON
`null`; unknown observation states are the string `unknown`.

| Top-level field | Type / meaning |
| --- | --- |
| `schema_version` | `agent-session.mail-audit.v1` |
| `machine` | Configured machine identifier |
| `snapshot_at`, `observation_finished_at` | UTC RFC3339 timestamps delimiting this page's observation interval |
| `older_than_seconds` | Effective threshold |
| `partial` | Boolean; one or more store sources could not be observed |
| `sources` | `inbox`, `notifications`, `outbox`: each `available`, `invalid`, `unsupported`, or `unavailable` |
| `records` | Ordered array of the record objects below |
| `next_cursor` | Resume string, or `null` at end |

| Record field | Type / meaning |
| --- | --- |
| `source` | `inbox` or `outbox`; terminal retained entries remain `outbox` |
| `message_id` | Stable message identifier |
| `sender` | Session address `{machine, session_id, session_incarnation}`; service address `{kind:"service", machine, service_id, service_generation}`; `null` for malformed historical sender metadata |
| `recipient` | Exact `{machine, session_id, session_incarnation}` |
| `category` | Sender category, default `uncategorized` |
| `sent_at`, `persisted_at`, `expires_at` | UTC timestamps or `null` |
| `unread_age_seconds` | Nonnegative integer since destination ingress, only for live unread mail |
| `end_to_end_latency_seconds` | Signed destination ingress minus source send time, or `null`; clock skew may make this negative |
| `mailbox_state`, `mailbox_revision` | `unread`, `read`, `acknowledged`, `expired`, `unknown` and revision; `null` for source-only records |
| `delivery_state` | `queued`, `delivered`, `rejected`, `delivery-unknown`, `unknown`; `null` for inbox records |
| `attempts`, `next_retry_at` | Source attempt count and queued retry time, or `null` |
| `last_attempt_at`, `state_changed_at` | Source attempt start / most recent delivery state transition time, or `null` |
| `reason_code` | Allowlisted federation reason or `unknown`, `null` when absent |
| `recipient_status` | `{runtime, broker, current_incarnation, heartbeat_at, heartbeat_fresh}` |
| `notification` | Exact-incarnation receipt described below, or `null` |
| `resume_carry` | Inbox mail moved by same-session resume: `{original_recipient_incarnation, from_incarnation, carry_count, carried_at, carried_at_epoch}` (UTC RFC3339 / Unix seconds); `null` otherwise and for outbox records |
| `anomalies` | Array of stable anomaly codes; empty for healthy records |

`recipient_status.runtime` is `running`, `stopped`, `missing`, or `unknown`.
It uses existing exact runtime identity evidence; an absent local session
record is `missing`, but invalid/unavailable records are `unknown`.
`broker` is `ready`, `stopped`, `lost`, `starting`, or `unknown`; only a broker
matching the current session incarnation is used. `heartbeat_fresh` uses the
existing exact-incarnation heartbeat check and is `null` without that broker.
A stale heartbeat alone never establishes stopped/deleted state. Remote source
rows always carry unknown recipient status; collector failure or an offline
host cannot establish recipient deletion.

A `notification` is joined on recipient ID **and incarnation**, using the
newest `(generation, updated_at_epoch)` receipt when historical entries coexist.
It has `state`, `generation`, `notified_generation`, `queued_at`, `attempted_at`,
`updated_at`, `next_retry_at`, and `reason_code`. States are `queued`,
`attempting`, `prompt_submitted`, `attempt_unknown`, `undeliverable`, or
`unknown`. Notifications can coalesce multiple messages: the receipt is
recipient-generation evidence, not proof that one message was read/executed.
Notification timestamp zero/missing means `null`; the query never fabricates a
migration time or rewrites the stored receipt.

## Classification and joining

| Code | Condition |
| --- | --- |
| `overdue-unread` | Live unread destination mail has ingress age greater than threshold |
| `recipient-stopped` | Live unresolved inbox mail targets the current incarnation and its exact runtime or broker is stopped |
| `recipient-missing` | Live unresolved inbox mail targets a provably absent local session record |
| `incarnation-mismatch` | Live unresolved inbox mail targets a different incarnation from the current local record |
| `queued-overdue` | Source delivery remains queued longer than threshold since source submission |
| `rejected` | Source terminal admission failure |
| `delivery-unknown` | Source cannot confirm delivery (including an unrecognized delivery state) |
| `notification-failed` | Live unresolved inbox mail has exact-incarnation `attempt_unknown` or `undeliverable` notification, except the normal `hook-delivered` fallback |

Unresolved inbox mail means live `unread` or `read`; acknowledged/expired mail
does not raise recipient or notification anomalies. Multiple flags coexist.
Expired-but-unpruned mail projects `expired` without altering stored state.

Collectors query `--include-healthy`, then join source/destination observations
by message ID **and both complete endpoint incarnations**. Keep source send,
destination ingress, delivery, mailbox and notification states separate.
`delivered` means destination persistence; it never overrides destination
`unread`. A stopped-target submission may remain queued/rejected at the source
without creating any destination mail. Accepted-then-stopped mail is a separate
inbox orphan. Ordinary local submissions rejected synchronously and never
persisted have no audit record; callers retain the submission error themselves.

Records sort by a stable metadata key containing source, message ID and both
addresses. Cursors resume strictly after that key and bind the query filters.
Candidates are cursor-filtered and ordered before full record projection and
recipient probes; projection stops after one page plus a lookahead. Stateless
CLI pages still reread the bounded stores, so collectors should use limit 100
for larger scans. Cursors are body-free strings, treated as opaque by consumers, and are
URL-encoded for HTTP. Each page reads independent atomic file snapshots;
pages are **not a transaction** across changing inbox/journal/runtime state.
Concurrent insertion before the cursor or changing anomalies can require a
fresh scan. Consumers retain the observation interval and restart periodic
scans; they must not treat a partial/failed scan as a clean host.

## Retention and compatibility

Readers accept existing registry v1/v2 and federation journal v1/v2/v3 plus the
new timestamp-bearing `agent-session.federation-journal.v4`. Missing stores are
empty; corrupt/unreadable/unsupported stores produce `partial` with a fixed
source code, leaving the other source observable. Store reads retain existing
private-file checks and byte bounds. Retention limits and delivery behavior are
unchanged: the audit only reports records still retained by their owners.

New source writers persist submission time through terminal compaction and
record each attempt start and actual state transition. New queued submissions
start their transition time at submission. Compaction preserves these times.
Expiry-only finalization records a state transition but no new relay attempt
time. The existing retry-accounting `attempts` count is preserved, including
its expiry-finalization increment; it is not proof of network contact.
Historical terminal entries lacking submission/attempt/transition timestamps
project `null`; expiry never serves as a substitute send time. A journal with
these new fields writes v4 so older strict readers fail closed rather than
silently discarding metadata. Rollback requires a version that understands v4;
there is no automatic downgrade or timestamp inference.

## Validation

The metadata fixture covers overdue unread, stopped/missing/old targets,
source pending/rejected deliveries, historical unknown times, notification joins,
healthy/acknowledged rows, pagination, and body/token/hash/path/reason canaries.
Unit tests cover threshold boundaries, unknown/unsupported sources, notification
uncertainty, cursor validation, and terminal timestamp compaction. CLI and daemon
tests compare private store bytes before/after queries and exercise operator
access boundaries. The separate-root federation fixture exercises a submission
to a stopped fixture broker and accepted-then-stopped inbox mail without
fabricating an unread destination entry for rejected/pending submission.
