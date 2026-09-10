# Session Retitle v3

Session Retitle v3 is the additive, daemon-owned protocol for reliable retitling
of long-running sessions. It replaces transcript-tail sampling with bounded
semantic memory and incremental provider-history projection. Total transcript
size MUST NOT affect a fresh manual Retitle operation's daemon latency, stored
state size, or provider input size.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are normative.

## Compatibility and ownership

The capability identifier is `agent-session.session-retitle.v3`. The daemon
owns provider-history access, semantic reduction, title rendering, provider
execution, durable receipts, fencing, and recovery. An edge or browser MUST NOT
reconstruct title context or receive the private semantic memory.

V3 is additive to [Session Retitle v2](session-retitle-v2.md):

- `/sessions` advertises `data.capabilities.session_retitle_v3 = true` only
  when all v3 routes and schemas in this document are implemented. It continues
  to advertise `session_retitle_v2` while v2 is available.
- A new edge with an old daemon MUST select v2. An old edge with a new daemon
  continues to use v2. A new edge selects v3 only after observing the v3
  capability from that same daemon instance.
- V3 state is stored under its own marker and MUST NOT alter or reinterpret a
  v2 receipt. V2 activity can change the visible title and title revision, so a
  later v3 mutation still has to satisfy the current v3 fences.
- Disabling v3 selection is a safe rollback. It MUST NOT delete v3 memory or
  stop, restart, or replace a provider tmux session.

## Authentication and routes

Every route requires the `agent-session serve` bearer and uses the outer
`cli.agent-session.serve.v1` envelope. Successful data is under
`data.{machine,retitle}`. No route accepts a session capability as a substitute
for the server bearer.

| Method and path | Contract |
| --- | --- |
| `GET /retitle/v3/readiness` | Machine-level v3/provider readiness |
| `GET /sessions/{id}/retitle-v3/readiness` | Session memory and history freshness |
| `POST /sessions/{id}/retitle-v3` | Strict mutation/admission request |
| `GET /sessions/{id}/retitle-v3/operations/{operation_hash}` | Durable operation reconciliation |

Machine readiness and session readiness are deliberately separate. A ready
provider does not prove that a particular session has current semantic memory.
Likewise, a session with usable cached memory can remain useful while the title
provider or history source is degraded.

## Machine readiness

`GET /retitle/v3/readiness` returns schema
`agent-session.session-retitle.readiness.v3`, capability
`agent-session.session-retitle.v3`, and these fields:

- `status`: `ready`, `degraded`, or `unavailable`;
- `reason_code`: a stable content-free code;
- `next_action`: a stable operator/client action.

Machine readiness MUST describe v3 execution/provider availability only. It
MUST NOT claim a session memory revision, usable memory, pending operation, or
history freshness. Clients obtain those facts from the session route.

The initial stable machine mappings are:

| `status` | `reason_code` | `next_action` |
| --- | --- | --- |
| `ready` | `ready` | `none` |
| `degraded` | `provider_unavailable_memory_supported` | `use_cached_memory_or_restore_provider` |
| `unavailable` | `retitle_v3_unavailable` | `restore_retitle_v3` |

`degraded` means cached session memory can still serve the memory-first path;
it does not assert that any specific session has such memory.

## Session readiness

`GET /sessions/{id}/retitle-v3/readiness` returns the same schema and capability
plus:

- `status`: `ready`, `catching_up`, `stale`, `degraded`, or `unavailable`;
- `provider_status`: `ready` or `unavailable` for the configured title
  provider;
- `context_status`: `ready`, `catching_up`, `stale`, `degraded`, or
  `unavailable` for semantic memory/history freshness;
- `title_status`: `current`, `stale`, `degraded_cached`, or `missing`;
- `reason_code` and `next_action`;
- monotonic `memory_revision`;
- `usable_memory`, indicating that an origin or active objective exists;
- `pending_operation`, indicating a durable non-terminal operation;
- optional opaque SHA-256 `cursor_fence_hash`, computed from the complete
  durable cursor rather than exposing its segment, source, or byte offset.

`context_status` is the bounded freshness projection. `cursor_fence_hash`
allows a client to distinguish progress without learning a provider session
ID, transcript path, segment, offset, or content. It MUST rotate when the
cursor changes or source continuity is lost and remain stable for an unchanged
cursor.

| `status` | Meaning | Representative reason | Action |
| --- | --- | --- | --- |
| `ready` | Usable memory is caught up to verified history | `ready` | `none` |
| `catching_up` | Bounded projection has more complete records to consume | `memory_not_initialized` or `history_catching_up` | `refresh_memory` |
| `stale` | New history or a source discontinuity was observed | `history_advanced` or `history_stale` | `refresh_memory` |
| `degraded` | Usable memory exists, but the history/provider refresh path failed | `history_read_degraded` or `degraded_cached` | `use_cached_memory_or_retry` |
| `unavailable` | No usable memory can be produced from current evidence | `provider_history_unavailable` or `memory_unavailable` | `restore_provider_history` |

Readiness is an observation, not a mutation fence. A caller MUST use revisions
returned by the current session projection when constructing the POST request.

## Strict request

The request schema is `agent-session.session-retitle.request.v3`. Unknown
fields are rejected.

```json
{
  "schema_version": "agent-session.session-retitle.request.v3",
  "trigger": "manual",
  "idempotency_key": "manual-retitle-0001",
  "expected": {
    "session_incarnation": "opaque-launch-id",
    "title_revision": 3,
    "memory_revision": 12
  }
}
```

`trigger` is `manual` or `automatic`. `idempotency_key` is 8-128 printable,
non-space ASCII bytes. The daemon permanently binds the key to the complete
request digest for as long as its bounded receipt is retained. A changed
request under the same manual key is a conflict.

`expected` is required for both triggers. A manual request supplies
`session_incarnation`, `title_revision`, and `memory_revision`, and omits
`activity_revision` and `provider_turn_id`. An automatic request additionally
requires both of those fields:

```json
{
  "schema_version": "agent-session.session-retitle.request.v3",
  "trigger": "automatic",
  "idempotency_key": "auto-opaque-digest",
  "expected": {
    "session_incarnation": "opaque-launch-id",
    "title_revision": 3,
    "memory_revision": 12,
    "activity_revision": 91,
    "provider_turn_id": "opaque-provider-turn"
  }
}
```

An automatic operation identity is deterministic for the session incarnation
and provider turn. Changes in incidental polling or scheduling state MUST NOT
fan one provider turn out into multiple operations. Raw provider turn IDs are
accepted only on the authenticated request and MUST be hashed before durable
storage, response projection, or logging.

## Operation response

The response schema is `agent-session.session-retitle.v3`. A POST or operation
GET returns one operation projection:

```json
{
  "schema_version": "agent-session.session-retitle.v3",
  "capability": "agent-session.session-retitle.v3",
  "status": "terminal",
  "outcome": "committed",
  "changed": true,
  "operation_hash": "sha256:...",
  "memory_revision": 13,
  "title_revision": 4,
  "session_incarnation": "opaque-launch-id",
  "readiness": "ready",
  "title": "Reliable long-session retitle",
  "admission_fence": {
    "session_incarnation": "opaque-launch-id",
    "title_revision": 3,
    "memory_revision": 12
  },
  "result_fence": {
    "session_incarnation": "opaque-launch-id",
    "title_revision": 4,
    "memory_revision": 13
  },
  "current_fence": {
    "session_incarnation": "opaque-launch-id",
    "title_revision": 4,
    "memory_revision": 13
  },
  "result_is_current": true,
  "started_at": "2026-09-09T00:00:00Z",
  "finished_at": "2026-09-09T00:00:01Z",
  "duration_bucket": "250_999_ms",
  "provider_attempts": [
    {
      "provider_kind": "codex_subscription",
      "model_label": "gpt-5.6-luna",
      "outcome": "success",
      "started_at": "2026-09-09T00:00:00Z",
      "finished_at": "2026-09-09T00:00:01Z",
      "duration_bucket": "250_999_ms"
    }
  ]
}
```

`status` has exactly two values:

- `accepted`: durable work remains. `outcome`, `changed`, `finished_at`, and
  `result_fence` MUST be absent. The response's `admission_fence` is immutable.
- `terminal`: no work remains for this operation. `outcome`, `changed`,
  `finished_at`, and `result_fence` MUST be present and immutable on every
  replay.

The top-level `memory_revision`, `title_revision`, `session_incarnation`,
`readiness`, and optional `title` are current observations sampled for this
response and equal `current_fence`. They can advance after an operation becomes
terminal. They MUST NOT be used to reinterpret the operation result.
`admission_fence` identifies what was admitted; `result_fence` identifies
exactly what the terminal operation committed or preserved.
`result_is_current` is true only when `result_fence` exactly equals
`current_fence`. It is false for every accepted operation and for a terminal
result superseded by later session state.

| `status` | `outcome` | `changed` | Meaning |
| --- | --- | --- | --- |
| `accepted` | absent | absent | Catch-up, queued refresh, or safe provider work remains |
| `terminal` | `committed` | `true` | The fenced title mutation committed |
| `terminal` | `unchanged` | `false` | Current memory rendered the existing title |
| `terminal` | `degraded_cached` | `false` | Refresh failed, but usable memory and the prior title were preserved |
| `terminal` | `terminal_failure` | `false` | No usable memory/title exists and the operation cannot produce one |

An identical retry or GET returns the original outcome, not a separate
`replayed` outcome. Reconciliation therefore does not change result semantics.

## Memory-first behavior

A manual Retitle with current, usable memory MUST complete from a deterministic
local render without invoking a title provider. It may commit a changed title
or return `unchanged`. Its work is independent of total transcript size.

If memory is stale, the daemon performs one bounded incremental refresh. It MAY
complete synchronously when that refresh reaches a safe commit point within
the request budget. Otherwise it durably admits the operation and returns HTTP
`202` with `status = accepted`; the caller polls the operation route.

A provider MAY be invoked only for a candidate semantic change that cannot be
rendered from already accepted memory. A current-memory manual request MUST NOT
invoke a provider merely to reproduce its existing title. Primary and fallback
execution follow the bounded taxonomy below.

The first automatic title MUST be provider-authored. The daemon MUST NOT expose
its private deterministic `objective:` projection as the session title. Image
transport scaffolding and image-reference markers are excluded from the
semantic projection supplied to that provider. A first automatic result MUST
set a non-empty topic and MUST reject internal projection prefixes,
separator-list output, and image-reference markers as malformed provider
output before commit or fallback selection.

When refresh or provider evaluation fails and usable memory plus a prior title
exist, the daemon preserves both and terminates as `degraded_cached`. The
provider failure MUST NOT turn that case into an HTTP error or a generic UI
failure. `terminal_failure` is reserved for a session with no usable memory or
title to preserve.

## Private semantic memory

The managed session record stores one private marker named
`session_retitle_v3`, schema `agent-session.session-retitle-state.v3`. Its
serialized JSON MUST be at most 16 KiB. The reducer MUST fold or evict eligible
old entries before a normal update reaches the bound; normal long-session
growth MUST NOT fail merely because the transcript grew.

The marker contains:

- a semantic projection version. A daemon that changes prompt sanitization MUST
  rebuild an older projection from provider history before sending it across a
  title-provider boundary; operation receipts survive that rebuild;
- monotonic `revision`;
- immutable `origin`, set from the first eligible sanitized human prompt;
- `active_objective`, initially the origin and replaceable only by an explicit
  sanitized human objective pivot;
- `current_activity`, which assistant progress may update;
- bounded `milestones`, `decisions`, `blockers`, and `journey` ledgers;
- bounded source `segments` and the current incremental `cursor`;
- last semantic turn and delta hashes;
- current readiness/freshness state;
- bounded durable operation receipts.

Individual semantic text fields are capped at 320 Unicode scalar values. Each
ledger retains at most six entries; segments and operation receipts retain at
most eight entries each. Eviction MUST prefer the oldest terminal receipt and
MUST NOT evict the active non-terminal operation.

The origin is never automatically replaced. Routine human follow-ups update
the journey but not the active objective. Only a deterministic, explicit pivot
cue in a human-submitted turn can replace `active_objective`. Assistant,
developer, system, tool, compact-summary, generated continuation, and terminal
output can update neither `origin` nor `active_objective`. Assistant text MAY
update activity or a bounded ledger after sanitization.

The provider input is a deterministic JSON projection of the accepted memory
and MUST be strictly smaller than 16 KiB. It excludes cursor, segment, receipt,
timestamp, path, credential, environment, and raw provider identity fields.
Repeated rendering of the same memory revision MUST produce identical bytes.

## Incremental history and discontinuity

The durable incremental cursor contains `source_id`, `segment_id`, `offset`,
`continuity_hash`, and `discarding_oversized_line`. These fields are private.
`source_id`, `segment_id`, and `continuity_hash` are opaque SHA-256 values.

The projector:

1. Verifies source identity, file length, and the continuity hash at the old
   offset.
2. Reads at most 1 MiB plus one byte for one refresh step.
3. Parses only complete provider records. It does not advance past an
   incomplete trailing line.
4. Discards an oversized record across bounded pages without buffering more
   than the line limit, then resumes at the following complete record.
5. Redacts and reduces eligible semantic messages and advances the cursor in
   the same locked memory commit.

An append preserves the segment. Rotation, truncation, compaction, prefix
rewrite, source replacement, or a failed continuity hash creates a new segment
with `discontinuity = true`. Existing origin/objective memory is preserved.
Readiness becomes stale or degraded until bounded catch-up on the new segment
converges; the old cursor is never advanced under the new source identity.

The catalog cache MUST be invalidated or bypassed when a verified source change
would otherwise hide the new segment. Resume binds the existing managed session
identity to the newly verified provider-history source; it does not reset
semantic memory.

## Fences and atomic commits

V3 uses compare-and-swap fences at every durable boundary.

Admission checks the requested session incarnation, title revision, and memory
revision. Automatic admission also checks a minimum activity revision and the
exact provider turn. A newer activity revision is acceptable only while the
same provider turn remains authoritative.

An incremental memory commit rechecks:

- managed-session identity and incarnation;
- title and memory revisions;
- source segment and complete old cursor;
- prior semantic turn and delta hash;
- the admitted operation identity and request digest.

A title/result commit additionally rechecks the complete current history
cursor, source segment, semantic delta hash, activity revision, and provider
turn. Any newer history delta, different current turn, incarnation change,
title change, or memory change rejects the old result before title mutation.
The one exception is a missing first title produced by an automatic operation:
assistant progress from the same provider turn MAY advance memory while the
provider runs, but the session incarnation, zero title revision, execution
claim, provider turn, and original active objective MUST still match.

Cursor advance, reduced memory, receipts, readiness, and any deterministic
title change MUST be written atomically under the existing session-record lock.
A crash before that write leaves the old cursor and memory authoritative. A
retry reprocesses the same delta once. A crash after it replays the receipt and
does not skip or duplicate the turn. A provider result is never allowed to
commit after its fence has become stale.

## Operation manager, coalescing, and recovery

There is one daemon operation manager and at most one executing Retitle job per
session. Its queue and durable receipts obey these rules:

- A manual operation has priority over every queued automatic operation. It
  runs next at the first safe boundary; it does not cancel an in-flight durable
  commit.
- Queued automatic work is replaceable. A newer authoritative provider turn
  supersedes older queued automatic work for that session. Superseded work
  terminates content-free and MUST NOT invoke a provider or mutate the title.
- Identical automatic scheduling observations coalesce to one deterministic
  operation. Identical manual requests join or replay their operation.
- Provider queue admission and provider execution have separate bounded
  deadlines. Waiting for a session slot does not consume a second provider
  execution attempt, and a provider call never receives a renewed full budget
  after queueing.
- Receipt stage, timestamps, fence, and attempt state are durable. After daemon
  restart, a non-terminal operation is adopted under the same per-session gate
  and resumes from its last proven safe state.
- An `accepted` response is non-terminal and remains scheduled for bounded
  reconciliation; polling it does not consume a provider-failure attempt.
- A non-retryable continuation failure MUST atomically terminalize its durable
  receipt before releasing the operation gate, so later scheduling or restart
  adoption cannot recreate the same failed work.
- Recovery never advances an unverified cursor, repeats a completed title
  commit, or retries a non-transient provider failure against unchanged input.

Primary/fallback invocation is at most one attempt each for one unchanged
input. Backoff retries are reserved for explicitly transient timeout,
unavailable, rate-limit, or quota classes and retain a durable attempt bound.
Malformed output, missing content, JSON parse, and schema validation are
terminal for that provider/input; the daemon may proceed once to a distinct
configured fallback. If an automatic operation has no usable title to preserve,
these provider failures keep the durable operation non-terminal and MAY retry
the provider chain under the same operation hash up to the global automatic
attempt bound. Exhaustion is terminal. This initial-title exception prevents a
single transient or malformed response from permanently leaving only the
session-ID fallback visible.

## Provider attempt observations

Every provider-bearing terminal operation retains and returns an ordered
`provider_attempts` array with at most two entries. Array position zero is the
primary; position one, when present, is the fallback. Every entry contains
only:

- `provider_kind`: `codex_subscription`, `openai_compatible`, or `command`;
- optional validated, non-credential-shaped `model_label`;
- `outcome`;
- optional `failure_stage` and `failure_class`;
- RFC 3339 `started_at` and `finished_at`, sampled around that exact provider
  attempt;
- `duration_bucket`.

Stable outcomes are `success`, `account_missing`, `api_key_missing`, `timeout`,
`unavailable`, `rate_limited`, `quota_exceeded`, `deadline_exhausted`,
`malformed_response`, and `worker_failed`. Stable stages are
`provider_admission`, `provider_worker`, `provider_setup`, `provider_call`,
`provider_response`, `provider_parse`, `provider_budget`, and `provider`.
Unknown internal stages collapse to `provider`.

For malformed responses, stable failure classes distinguish at least
`missing_message`, `json_parse`, `schema_validation`, `response_read`, and
`response_encoding`. Unknown internal classes collapse to `malformed_response`
or `failed`. The terminal operation also projects the final `failure_stage` and
`failure_class` for simple clients; consumers that display the provider chain
use the ordered array.

Operation durations use the v2 coarse buckets: `under_10_ms`, `10_49_ms`,
`50_249_ms`, `250_999_ms`, `1_4_s`, `5_29_s`, `30_119_s`, and `120_s_plus`.
Raw elapsed values are not retained.

## HTTP status and stable failures

| HTTP | Condition |
| --- | --- |
| `200` | Readiness/operation GET succeeded, or POST returned a terminal operation |
| `202` | POST durably admitted a non-terminal operation |
| `400` | Invalid JSON, schema, trigger, key, field combination, or operation-hash shape |
| `401` | Missing or invalid server bearer |
| `404` | Session or retained operation does not exist |
| `409` | Idempotency, incarnation, revision, turn, history, cursor, or result fence conflict |
| `422` | Existing session cannot supply required semantic/history evidence and has no cached result |
| `503` | Required provider/history dependency is unavailable and no degraded cached result is possible |
| `500` | Sanitized unexpected storage or worker failure |

Stable v3 error codes include:

- `invalid-retitle-v3-request`;
- `retitle-v3-idempotency-conflict`;
- `retitle-v3-state-conflict`;
- `retitle-v3-turn-conflict`;
- `retitle-v3-history-conflict`;
- `retitle-v3-history-stale`;
- `retitle-v3-history-unavailable`;
- `retitle-v3-history-degraded`;
- `retitle-v3-memory-not-ready`;
- `retitle-v3-memory-invalid`;
- `retitle-v3-memory-version-unsupported`;
- `retitle-v3-memory-too-large`;
- `retitle-v3-operation-not-found`;
- `title-revision-overflow`.

Each error has typed, content-free `details.retryable`,
`details.next_action`, and `details.recovery.{strategy,safe_to_retry}`. Clients
branch on code and typed details, never message text.

## Privacy boundary

The private memory is durable derived content and receives the same protection
as the managed session record. Redaction occurs before persistence and before
provider input construction, not merely at the HTTP boundary.

The memory, receipt, response, structured stderr/journald event, and retained
test/deployment evidence MUST NOT contain:

- raw prompts, transcript excerpts, assistant/provider/terminal output, or
  generated model response bodies;
- raw provider turn or session IDs;
- idempotency keys;
- credentials, authorization headers, environment values, private keys, or
  credential-shaped model labels;
- provider commands, absolute transcript/state/project paths, or raw cursors.

HTTP may expose the user-visible title, bounded readiness fields, opaque hashes,
and the content-free provider attempt taxonomy. It MUST NOT expose origin,
active objective, current activity, ledgers, segments, or provider input.

Complete injected instruction regions, provider context blocks, image payloads,
tool results, generated user records, compact summaries, and developer/system
messages are excluded before semantic reduction. Sanitization removes private
paths and token-shaped values before any retained summary is formed.

## Limits and performance

- Private semantic marker: at most 16 KiB serialized JSON.
- Provider input: strictly less than 16 KiB.
- One incremental refresh read: at most 1 MiB plus one byte.
- Semantic fact: at most 320 Unicode scalar values.
- Each ledger: at most six entries.
- Source segments: at most eight.
- Durable receipts: at most eight, without evicting non-terminal work.
- Provider attempts per unchanged input: one primary plus one fallback.
- Fresh usable-memory daemon work: p95 at most 250 ms, excluding HTTP/client
  transport and any separately admitted asynchronous refresh.

Fresh-memory latency, marker size, provider input size, and operation lookup are
bounded independently of transcript bytes and record count. The routine
production-shaped regression is approximately 200 MiB/44,000 records. The
virtual 1 GiB case remains an explicit stress test rather than a routine local
gate.

## Acceptance matrix

| Case | Required evidence |
| --- | --- |
| Event-dense long history | V2 loses the intermediate human objective within its bounded tail; v3 incrementally recovers it from the generated approximately 200 MiB/44,000-record history |
| 1 GiB stress | Explicit virtual test retains constant fixture allocation and does not change state/provider-input/fresh-latency bounds |
| 1,000 turns | Origin remains unchanged; ledgers, segments, receipts, marker, and provider input remain bounded |
| Human pivot | Explicit sanitized human pivot changes `active_objective` while preserving `origin` |
| Assistant non-pivot | Assistant/developer/system/tool/generated content cannot alter either objective |
| Restart/resume | Durable cursor, memory revision, operation, and objective invariants rehydrate without a full-history retitle |
| Append | Same segment advances only from the verified complete-line cursor |
| Partial/oversized record | Cursor waits for a complete line and boundedly discards an oversized line before resuming |
| Rotation/compaction | New segment is appended, old memory survives, discontinuity is visible, and bounded catch-up restores `ready` |
| Stale CAS | New incarnation/title/memory/history/activity/turn evidence rejects an older result before mutation |
| Crash points | Injection before cursor commit, after reduction, and before title commit converges idempotently without a skipped or duplicate turn |
| Automatic coalescing | Repeated same-turn scheduling invokes one operation; newest queued turn supersedes older queued automatic work |
| Manual priority | Manual work is selected ahead of queued automatic work and remains reconcilable by operation hash |
| Provider taxonomy | Primary/fallback timeout, unavailable, quota, rate-limit, missing-message, JSON-parse, and schema-validation remain distinguishable without content |
| Degraded cached | Failed refresh with usable memory preserves title and returns terminal `degraded_cached` |
| No cache | Failed refresh with no usable memory returns typed terminal failure/error and never invents a title |
| Fresh manual | Repeated Retitle completes without provider inference, stays under the 250 ms p95 daemon target, and preserves objective |
| Mixed versions | old-edge/new-daemon, new-edge/old-daemon, and v2-only combinations select a safe supported path |
| Privacy canaries | Session record, HTTP, operation replay, structured logs/journal, provider attempt evidence, and retained artifacts contain none of the forbidden values |
