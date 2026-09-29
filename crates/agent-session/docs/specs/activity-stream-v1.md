# Activity stream v1

## Purpose

`agent-session serve` exposes clock-safe observation anchors and near-real-time
activity metadata without coupling provider hooks to network clients. This is an
additive contract: existing clients may continue polling `GET /sessions`, and
the terminal attach WebSocket is unchanged.

## Session observation anchor

Successful `GET /sessions` responses retain
`schema_version: cli.agent-session.serve.v1` and add
`data.observed_at`, an RFC 3339 daemon timestamp sampled after the returned
session list is assembled and immediately before response serialization.
Consumers use that value as the wall-clock anchor for the adjacent state, then
advance elapsed time from a local monotonic clock. The existing poll remains
the authoritative reconciliation path after stream gaps and for old daemons.

## Endpoint and authentication

`GET /activity/events` returns `text/event-stream`. It requires the same Bearer
token as write and attach endpoints: a missing/invalid token returns 401, and a
daemon without a configured token returns 503. Tokens never appear in URLs or
event data. Responses use `Cache-Control: no-cache, no-transform` and
`X-Accel-Buffering: no`.

The broker lifecycle is explicit: `starting` advances to `ready` only after the
filesystem watcher and initial session snapshot both succeed; a watcher callback
error or later snapshot-collection failure moves it permanently to `degraded`.
On degradation, an existing stream receives one full `reset` and then closes,
heartbeats stop, and new requests return 503 `activity-stream-unavailable`.
The daemon preserves all existing endpoints, so clients fall back to session
polling rather than treating a degraded stream as healthy. `GET /sessions` and
the broker use the same injected session snapshot source, keeping polling and
stream projection aligned.

Each degraded terminal `reset` receives a new sequence greater than every
previous frame from that stream, so consumer deduplication cannot discard the
reconciliation signal. Its `observed_at` is deliberately the anchor of the last
successful snapshot collection represented by `sessions`, not the later time
when degradation was detected. A subsequent successful `GET /sessions` carries
its own post-assembly anchor and therefore outranks that cached reset. Consumers
must not treat a reset's delivery time as evidence that its cached sessions are
fresh.

Every SSE frame uses:

- `id: <stream_id>:<sequence>`
- `event: snapshot`, `heartbeat`, or `reset`
- one JSON object in `data`

The JSON shape is:

```json
{
  "schema_version": "agent-session.activity-stream.event.v1",
  "type": "snapshot",
  "stream_id": "opaque-daemon-boot-id",
  "sequence": 42,
  "machine": "sympoies",
  "observed_at": "2026-07-11T17:00:00Z",
  "sessions": [
    {
      "id": "session-id",
      "turn_state": {
        "schema_version": "agent-session.turn-state.v1",
        "phase": "waiting",
        "phase_changed_at": "2026-07-11T16:59:00Z",
        "revision": 7,
        "source": {
          "kind": "provider_hook",
          "provider": "codex",
          "confidence": "authoritative"
        },
        "semantic_event": {
          "kind": "turn_completed",
          "observed_at": "2026-07-11T16:59:00Z"
        },
        "last_turn": {
          "provider_turn_id": "opaque-id",
          "started_at": "2026-07-11T16:58:00Z",
          "completed_at": "2026-07-11T16:59:00Z",
          "outcome": "completed"
        }
      }
    }
  ]
}
```

`snapshot` and ordinary `reset` frames contain a full list of daemon session
ids. `turn_state` is `null` when no valid activity snapshot exists. `heartbeat`
omits `sessions` and is emitted every 15 seconds. `stream_id` is stable for one
daemon process; `sequence` strictly increases across emitted frames.

A projected snapshot whose complete SSE wire frame would exceed 512 KiB is not
retained or broadcast. The producer emits this bounded content-free reset
instead:

```json
{
  "schema_version": "agent-session.activity-stream.event.v1",
  "type": "reset",
  "stream_id": "opaque-daemon-boot-id",
  "sequence": 43,
  "machine": "sympoies",
  "observed_at": "2026-07-11T17:00:00Z",
  "reason": "oversized_snapshot"
}
```

Only this exact `reset` reason permits `sessions` to be omitted. It is a
content-free invalidation, not an empty session list, and requires immediate
`GET /sessions` reconciliation. The reset is emitted once when state crosses
from bounded to oversized; later oversized refreshes update polling state but
do not repeat the reset or advance the stream sequence. A later bounded
snapshot is emitted as the recovery transition. Polling remains authoritative
and available.

Nested optional `turn_state` leaves use the same omission semantics as
`GET /sessions`: absent provider ids, progress timestamps, semantic events,
diagnostics, shadow observations, attention, current turn, and last turn are
omitted rather than serialized as `null`. The
session-level `turn_state: null` remains intentional and means no valid activity
snapshot. The exact multi-session Rust producer fixture consumed by downstream
contract tests is
[`tests/fixtures/activity/activity-stream-v1-multi-session.json`](../../tests/fixtures/activity/activity-stream-v1-multi-session.json).

## Replay, gaps, and backpressure

Without `Last-Event-ID`, a subscriber first receives the latest full snapshot.
A retained cursor for the current stream replays events whose sequence is
greater than the cursor. A malformed cursor, another daemon boot id, a cursor
beyond the current sequence, or an evicted cursor receives the latest full
state as `reset`. An exact cached full frame retains its existing identity;
otherwise a subscription recovery snapshot or reset receives a new global
sequence, enters replay history, and is broadcast to existing subscribers.
That gives every distinct emitted payload one stable event identity without
creating a sequence gap. The replay window retains at most 128 frames and at
most 512 KiB of pre-framed SSE wire bytes. Oldest frames are evicted until both
limits hold; if any sequence needed by a cursor was evicted, the subscriber
receives a reset. An oversized latest snapshot receives the content-free reset
described above.

Consumers deduplicate by `(machine, stream_id, sequence)`. A sequence gap or a
`reset` triggers immediate `GET /sessions` reconciliation. Degraded resets have
unique increasing sequences even when several subscribers stop concurrently.
The regular session poll remains active for convergence, daemon health, and
old-peer fallback.

The producer serializes each event payload and complete SSE frame once, caches
its wire length, and shares the same immutable frame across replay, broadcast,
and every subscriber. A single-frame broadcast slot prevents the queue from
retaining 32 large frames outside the 512 KiB replay budget; producers never
await a subscriber, and a lagged subscriber receives a cached reset from the
latest state. At most 64 concurrent SSE subscribers are admitted. Further
authenticated requests receive 429 `activity-stream-capacity`; disconnecting a
subscriber releases its permit and polling remains available throughout
saturation.

Filesystem notifications for `activity.json` and session lifecycle changes use
a capacity-one dirty bit. The first isolated refresh waits for a trailing 25 ms
quiet window. Under
a continuous burst, a refresh starts by the 250 ms cadence; after any refresh
starts, the next refresh cannot start for at least 250 ms. Notifications
arriving during a scan stay dirty and converge in a later rate-bounded refresh.

Shadow observations are populated asynchronously by the long-lived serve
session collector and do not make list or SSE refresh wait for tmux capture.
One-shot CLI list/status paths read the cache without launching detached work.
The sampler runs at most four captures concurrently, uses a shadow-specific
nonblocking lock, revalidates launch identity and generation before writing,
and never holds the provider activity-ingestion lock. Its own `activity.shadow.json`
writes are intentionally not broker refresh triggers; the next ordinary
provider/session refresh or authoritative four-second poll exposes the cached
diagnostic without a feedback rescan.

A notify event marked `need_rescan()` forces the same full snapshot collection
even when it has no relevant path. Removal or rename of the watched sessions
root first recreates the directory and replaces the recursive watcher before a
full refresh. If that root-loss invalidation cannot be queued or the watcher
cannot be re-armed, the broker degrades: existing streams receive their final
reset and EOF, heartbeats stop, and new stream requests receive the polling
fallback response. HTTP polling remains available throughout and is not the
normal transition source.

## Privacy boundary

Stream state is constructed from an allowlist. It may contain session id,
phase/timestamps/revision, provider source/confidence, the last semantic event
kind/time, an allowlisted diagnostic reason, opaque projected turn id, attention
kind/time/count/certainty, bounded shadow observer metadata, and outcome.
Forward-compatible unknown fields from durable snapshots are deliberately
excluded.

Prompt, response, command, tool payload, terminal output, transcript/config
paths or contents, and credentials are forbidden. By default provider hook
processes only perform their existing bounded local durable writes. A hook that
opts into the [provider hook ingress](#provider-hook-ingress) instead makes one
bounded loopback request to the daemon; neither transport waits for a
subscriber or performs network I/O for streaming.

## Provider hook ingress

`POST /activity/hook/v1` lets a provider hook report the same lifecycle payload
without write access to the state directory. It is the HTTP form of
`agent-session activity hook`: the daemon runs the identical normalization and
durable ingestion, so both transports accept exactly the same payload schema
and produce the same `turn_state` transition, activity diagnostic, and stream
refresh.

A hook opts in with `agent-session activity hook --agent <provider>
[--event <name>] --via http`; the default is `--via file`. The declaration
otherwise keeps its shape. The client reads the payload from stdin under the
same 64 KiB bound, reads `AGENT_SESSION_ID`, `AGENT_SESSION_RUNTIME_ID`, the
optional `AGENT_SESSION_ATTENTION_AUTHORITY`, and the capability named by
`AGENT_SESSION_CAPABILITY_FILE` from the managed runtime environment, and
reads the daemon's loopback URL from the owner-only
`<state-dir>/coordination/daemon-endpoint.json` that `serve` publishes at
startup. It needs read access to those two files and loopback network access,
and writes nothing. It connects without any HTTP proxy, bounds the request to
two seconds, stays silent, and always exits 0: like the file path, hook
telemetry is fail-open and never blocks a prompt, permission, or turn.

The request is `application/json` with the session capability in
`X-Agent-Session-Capability`. The operator bearer is neither required nor
consulted. The body rejects unknown fields:

```json
{
  "schema_version": "agent-session.activity-hook.v1",
  "session_id": "session-id",
  "session_incarnation": "runtime-launch-id",
  "agent": "claude",
  "event": "UserPromptSubmit",
  "attention_authority": "hook",
  "payload": "{\"hook_event_name\":\"UserPromptSubmit\"}"
}
```

`event` and `attention_authority` are optional and correspond to `--event` and
the runtime's attention-authority environment. `payload` is the exact provider
hook text the file path reads from stdin; a payload over 64 KiB or one that is
not valid JSON fails with the file path's own `provider-hook-too-large` or
`provider-hook-invalid` code. Selectors are bounded to 256 characters and the
whole body to the escaped payload bound.

Admission is fail-closed, in order:

1. Only a direct loopback peer is accepted. A non-loopback peer, missing peer
   information, or a `Forwarded`, `X-Forwarded-For`, or `X-Real-IP` header
   returns 403 `activity-ingress-forbidden`. This matters when the daemon is
   bound with `--allow-non-loopback`; it is not authentication, and a
   same-host TCP forwarder remains indistinguishable from a local caller.
2. A missing capability returns 401 `coordination-unauthorized`.
3. A malformed body, content type, schema version, or provider returns 400.
4. At most eight requests authenticate and ingest concurrently; further
   requests return 429 `rate-limited`.
5. The capability must authenticate the named session's ready, heartbeat-fresh
   broker, and `session_incarnation` must equal the incarnation it binds. A
   capability for another session, a replaced or stale incarnation, or an
   unknown capability returns 401 `coordination-unauthorized` without
   mutating state.
6. Each authenticated session draws from its own token bucket (burst 64,
   refilled at 20 requests per second) and may ingest at most two requests at
   once, so a session whose record lock is held elsewhere cannot occupy every
   shared slot. An exhausted bucket or a full session share returns 429
   `rate-limited`. Unauthenticated requests never draw from a session's
   bucket, so they cannot starve it.

Success returns the ordinary serve envelope with
`data.ingested`, which is `false` when the payload normalizes to no activity
event, exactly as the file path ignores it. Ingestion failures return the file
path's error code and record the same activity diagnostic. Responses never echo
the capability or payload.
