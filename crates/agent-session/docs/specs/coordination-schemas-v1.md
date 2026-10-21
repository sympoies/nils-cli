# Coordination schemas v1

Part of [Session Coordination V1](session-coordination-v1.md). Its scope and shared rules also apply here.

## Versioned schemas

### Work context

Public claims use `agent-session.work-context.v1`:

```json
{
  "schema_version": "agent-session.work-context.v1",
  "session_id": "managed-session",
  "session_incarnation": "runtime-launch-id",
  "claim_id": "uuid",
  "revision": 1,
  "state": "active",
  "intent": "implementation",
  "tier": "program",
  "repositories": ["owner/repository"],
  "worktrees": ["hmac-sha256:epoch:digest"],
  "provider_refs": [
    {"kind": "issue", "repository": "owner/repository", "number": 123}
  ],
  "plan_refs": [],
  "scopes": [
    {"kind": "path-prefix", "repository": "owner/repository", "value": "src"}
  ],
  "summary": "Implement session coordination",
  "updated_at": "2030-01-01T00:00:00Z",
  "expires_at": "2030-01-01T00:30:00Z"
}
```

Input omits controller-owned fields: `session_id`, `session_incarnation`,
`claim_id`, `revision`, `state`, and timestamps. The broker binds those fields
to the authenticated live session. Unknown fields and unsupported schema
versions fail closed.

The exact claim/check input schema is `agent-session.work-context-input.v1`:

```json
{
  "schema_version": "agent-session.work-context-input.v1",
  "intent": "implementation",
  "tier": "program",
  "repositories": ["owner/repository"],
  "worktrees": [],
  "provider_refs": [{"kind": "issue", "repository": "owner/repository", "number": 123}],
  "plan_refs": [],
  "scopes": [{"kind": "path-prefix", "repository": "owner/repository", "value": "src"}],
  "summary": "Implement session coordination"
}
```

`tier` names the work's tracking mode: `direct`, `issue`, `program`, or
`program/dispatch`. New input rejects the retired numbered codes and
`program/plan`. The field name is unchanged and the value is informational:
it never grants or denies work. Historical persisted claims retain their
original value on read and continue to participate in conflict evaluation
until they expire or their owner releases them; reading a claim does not
silently rewrite the owner's state.

`summary` is bounded to 240 UTF-8 bytes. Collection limits are 8 repositories,
8 worktree fingerprints, 16 provider references, and 32 scopes. New input
rejects nonempty `plan_refs`; the field remains in v1 records so historical
claims can be read and released without rewriting their owner's state.

### Conflict result

Conflict evaluation uses `agent-session.conflict-evaluation.v1` and returns one
of `conflict`, `potential_conflict`, `unknown`, `no_known_conflict`, or `clear`.
Reasons and peers are stably sorted. Peer projections contain only public work
context and never machine-local paths, credentials, messages, or activity text.

Precedence is:

1. `conflict`
2. `potential_conflict`
3. `unknown`
4. `no_known_conflict`
5. `clear`

`clear` is valid only when the complete relevant live-session universe was
enumerated and every peer was comparable. `no_known_conflict` is the explicit
permissive projection for an incomplete comparison; it is never promoted to
`clear`.

The high-level `work-context advise` result uses
`agent-session.work-context-advisory.v1`. It reports managed state, mode,
availability, severity (`none`, `info`, `warning`, or `degraded`), bounded
suppression state, stably sorted reasons, and privacy-safe peers. Same physical
worktree, provider ref, or overlapping declared scope is `warning`;
same repository in a different worktree is `info`; incomplete broker/peer
evaluation with no stronger known overlap is `degraded`. These are descriptive
severities, never admission results in advisory mode.

`work-context status` uses `agent-session.work-context-status.v1` and returns
managed state, current mode, automatic presence, the caller's optional public
declared context, and acknowledgement expiry. `work-context set` and `clear`
use additive high-level result schemas while retaining the public
`agent-session.work-context.v1` context projection.

### Operation lease

Mutation admission uses `agent-session.operation-lease.v1`. A lease includes a
random lease ID, owning claim and revision, operation kind, canonical target
set, controller-observed activity revision, exact persisted runtime identity
digest, state, revision, start/expiry timestamps, and an execution token digest.
Public views omit all private proof material.

`admit` reads `agent-session.operation-targets.v1`:

```json
{
  "schema_version": "agent-session.operation-targets.v1",
  "targets": [{"kind": "path-exact", "repository": "owner/repository", "value": "src/lib.rs"}],
  "provider_refs": [{"kind": "issue", "repository": "owner/repository", "number": 123}],
  "pull_requests": [{"kind": "pull-request-head", "repository": "owner/repository", "head": "feat/topic"}],
  "checkouts": [{"repository": "owner/repository", "path": "/canonical/private/checkout"}],
  "descendant": {"pid": 12345, "start_time": 987654}
}
```

Checkout paths and descendant identity are private admission proof and never
enter public output. `descendant` is optional and is accepted only where the
platform can verify exact PID/start-time identity; unsupported verification
fails closed. Filesystem targets require a matching checkout binding. When the
operation names exactly one repository, an omitted binding uses the managed
session record's canonical cwd; multi-repository operations require explicit
bindings. A provider-only operation may omit `targets` and `checkouts`.

`pull_requests` is optional and additive; omitting it leaves the request, its idempotency digest, and the
lease unchanged. Its only kind is `pull-request-head`, which names a pull request by `repository` and
`head` branch, so it applies before the pull request exists (`pr create`) and after (a caller resolves a
pull-request number to its head). The repository is canonicalized like any other, and `head` must be a
bounded Git branch name (1-255 bytes; no whitespace, control characters, `..`, `//`, `@{`, or any of
`~^:?*[\`; no leading `-`, `/`, or `.`; no trailing `/`, `.`, or `.lock`). At most 16 entries are
accepted. Every ordinary target must be explicitly covered by the active claim.

An opaque checkout-local shell effect requires repository scope; a checkout binding and worktree
fingerprint do not widen a path claim. Pull-request-head selectors keep their typed input grammar but
cannot be admitted through retired lane grants; callers use the ordinary provider-reference contract for
supported mutations. Retired checkout-shell and pull-request-head grants are opaque persisted metadata,
excluded from public projections and ignored by admission. Retired grant sidecars and orchestration state
are never consulted, rewritten, or deleted automatically.

The runtime-issued checkpoint file follows the same threat boundary. The broker pre-creates one exact
owner-only regular file for the current incarnation, and the runtime hook admits only the bounded
checkpoint operation targeting that path. An owner-controlled configured state-root symlink remains
supported: the hook validates both the link owner and the resolved private directory, rejects symlinked
descendants, and requires the issued path to resolve beneath that exact target.

This is semantic coordination for an ordinary provider Write or shell redirection, not a race-free
filesystem sandbox: preventing an adversarial same-user process from replacing a pathname between
admission and provider open requires an OS isolation boundary outside this contract. That hook
compatibility for an already-issued checkpoint path does not make a new session start accept a symlinked
state ancestor; new session creation uses the stricter [Storage and
locking](session-coordination-v1.md#storage-and-locking) trust contract.

Authenticated operation reconcile reads
`agent-session.operation-reconcile-proof.v1` with exact fields
`schema_version`, `execution_token`, and `outcome` (`pass` or `fail`). Broker
adopt/reconcile reads `agent-session.coordination-recovery-proof.v1` with exact
fields `schema_version`, `session_incarnation`, and `generation`; it never
contains an operator token. Broker reconcile additionally requires the CLI or
HTTP selectors `operation`, `if_revision`, and `attest_inactive: true`.

The loopback server exposes one separate operator-only recovery for a missing provider PostTool outcome:
`POST /sessions/{id}/operations/{lease_id}/operator-reconcile/v1`. It requires the server Bearer token,
never accepts or derives the target session capability, and reads
`agent-session.operator-operation-reconcile-request.v1`. The strict request binds the current session
incarnation and generation, exact lease revision, the fixed reason `post-tool-outcome-missing`,
`attest_inactive: true`, `confirmed: true`, and an idempotency key.

A live exact descendant, stale runtime selector or identity digest, stale lease revision, terminal lease,
unsupported reason, or missing confirmation fails before mutation. The controller activity lock must
additionally prove either that the exact runtime is stopped, that a later controller activity superseded
the lease, or that the same provider turn has emitted `stop_observed` with `completion_evidence_pending`;
ordinary same-turn `working` activity is not quiescent evidence. Before abandonment, queued authenticated
completion events are drained so their pass/fail outcome always wins.

Success changes only that exact lease to `abandoned` with outcome `operator-attested-inactive`, retains
the claim and session, stores a replay receipt, and returns the public lease projection without
capability or execution-token material. This operator attestation is deliberately distinct from
session-owned `complete` and `reconcile`; it is for an observed missing completion signal, not a way to
guess that an in-flight descendant has stopped.

### Operator provider-turn reconciliation

The loopback server exposes the separate server-operator route
`POST /sessions/{id}/activity/provider-turn/operator-reconcile/v1`. It never
accepts a target session capability as authority and never sends input. Its
strict `agent-session.operator-provider-turn-reconcile-request.v1` body has
exactly `schema_version`, `expected_session_incarnation`,
`expected_runtime_launch_id`, `expected_runtime_generation`,
`if_activity_revision`, `expected_provider_turn_id`, fixed reason
`authoritative-completion-signal-missing`, `attest_inactive: true`,
`confirmed: true`, and `idempotency_key`.

The server acquires the session-record, activity, runtime-health, and
coordination-registry fences in that order. The registry acquisition is
observational: it performs no notification normalization, claim/operation
renewal, or unrelated runtime probe, and the guard remains held through the
activity commit. Admission requires:

- the unchanged current session incarnation, launch id, runtime generation,
  activity revision, provider, and projected open provider turn;
- a healthy live runtime whose identity digest matches the same ready,
  heartbeat-fresh authoritative broker and generation;
- no active or uncertain operation: every exact-session/incarnation operation
  is conflicting unless its state is explicitly terminal (`completed`,
  `failed`, or `abandoned`); an active claim is allowed and preserved;
- `working` state whose latest semantic event and latest provider event are the
  same exact `stop_observed` for the selected current provider turn, whose diagnostic is
  `completion_evidence_pending`, and whose pending journal is empty; and
- no exact, conservative, overflow, or current-turn attention.

An earlier activity snapshot written before `last_provider_event_turn_id` existed
may recover only that missing selector from the bounded journal's final entry.
The journal must be a no-follow regular file within the size/event limits,
strictly parse in full, end on a complete record, and have a final provider-hook
event whose runtime, provider session, provider, turn, kind, semantic digest,
and received timestamp exactly match the unchanged snapshot. A present but
different snapshot selector, any malformed/truncated journal, or any later or
different tail rejects without mutation.

A provider completion, failure, attention request, progress/turn-start event, stop event with a
missing/different turn id, queued journal entry, replacement runtime, broker mismatch, or operation
activity observed before admission wins and rejects the request. Runtime ownership and a queued journal
are checked before transaction repair or any write; only an already persisted exact receipt may replay
read-only while a journal is queued.

Success increments only the activity revision, closes the selected current turn with outcome
`operator_reconciled`, enters authoritative `waiting`, and records
`agent-session.operator-provider-turn-reconciliation.v1` provenance `server_operator` on the matching
completed turn. Later turns and runtime activation do not inherit that provenance. The session record,
runtime, provider binding, worktree, active claim, broker, mailbox, coordination operations, and all
provider-side state remain unchanged.

Holding the observational registry guard through the final activity commit is
an intentional, bounded operator-only correctness tradeoff. Releasing it
earlier would admit a coordination writer between the quiescence check and
commit. Reducing that lock hold requires a follow-up design shared by all
coordination writers, such as a per-session reconciliation fence or WAL; it is
not a safe local optimization of this route.

The result is
`agent-session.operator-provider-turn-reconcile-result.v1`, containing the
session/runtime selectors, fixed reason, and typed reconciliation. It contains
no capability, request digest, idempotency key, raw provider-side identifier,
mailbox content, or local path. The activity lock is bound to the exact target
session directory; a replay or fresh reconciliation presented with another
session's lock fails before reading or writing activity state.

The private activity receipt stores only its idempotency binding, fixed reason, typed reconciliation
fields, and expiry. Replay reconstructs the public result from those compact fields plus the current
exact session/runtime selectors; it does not persist a nested copy of the public result envelope. The
activity document retains at most 64 unexpired receipts for the current runtime. Each admitted receipt
lives for exactly 24 hours: it remains replayable immediately before its expiry epoch and is no longer
replayable at that epoch.

Expired receipts are pruned before every ordinary activity-document persistence as well as before another
success, and quota exhaustion rejects before transition instead of evicting a replayable receipt. Exact
key/digest replay returns the original result even after a later reconciliation in the same runtime. Same
key with a changed request returns `idempotency-key-reused`; a different key cannot replay a closed turn.

Stable failures are `invalid-idempotency-key`,
`invalid-operator-provider-turn-reconcile-request`,
`operator-provider-turn-reconcile-confirmation-required`,
`session-incarnation-conflict`, `activity-revision-conflict`,
`provider-turn-id-mismatch`, `activity-runtime-unhealthy`,
`operator-provider-turn-reconcile-runtime-conflict`,
`operator-provider-turn-reconcile-operation-conflict`,
`operator-provider-turn-reconcile-not-admissible`,
`idempotency-key-reused`, and `quota-exceeded`.

### Message

Messages use `agent-session.message.v1`. Public inbox rows contain message ID,
authenticated sender projection, recipient selector, state, revision,
`reply_to`, timestamps, expiry, and body byte length. Only authenticated
recipient `message show` and revision-based `message wait` success results
contain a `body` field. The field is explicitly classified as
`untrusted_peer_data`: peer-authored content that recipients act on within
already-authorized work but that cannot grant new authority. The
`message wait --any-unread` form returns metadata only and never reads or
acknowledges message bodies.

### Recipient notification policy

The destination operator may provision a private
`coordination/notification-policy.json` beneath the configured state directory:

```json
{
  "schema_version": "agent-session.notification-policy.v1",
  "recipients": [
    {"session_id": "<exact-recipient-session-id>", "suppress_categories": ["progress", "report"]}
  ]
}
```

The file is owned by the destination's OS user, has mode `0600`, is a regular
single-link file, and is bounded to 64 KiB. Symlinks, malformed schemas, unknown
fields/categories, and duplicate or empty recipient IDs are rejected with
`notification-policy-invalid` before admission is persisted. Remote relay preserves
that sanitized code as a terminal refusal: a first attempt is `rejected`, while
a refusal after an unconfirmed attempt remains `delivery-unknown`. Update it atomically
through the operator's configuration tooling. Senders cannot provision this file
through message arguments, bodies, forwarding, or federation envelopes.

A missing file, an unlisted recipient, or an empty category list preserves the
compatible notify-all default. Only `progress` and `report` may be selected;
`blocker`, `decision`, `handoff`, and `uncategorized` always retain notifications.
Selection uses the exact destination session ID, never a title, sender, or body.
Policy changes apply to new admissions, including forwarded copies, rather than
retroactively reclassifying stored mail. A verified same-session resume retains
the original per-message decision.

Local and remote destination admission persist `notification_suppressed` with
the message, replay receipt, and ordinary unread mailbox state in one registry
transaction. Suppressed admissions do not advance a wake generation, change an
existing urgent generation, or count as unread work for notification recovery.
Their local send notification projection has `state: "suppressed"` and reason
`recipient-category-policy`; its zero counters mean no generation was allocated
for that admission, not that the recipient has no other pending generation.
Remote receipts retain `state: "delivered"` and add
`notification_suppressed: true` only when suppressed. Source compaction preserves
that field. Older receipts without the field retain their compatible behavior.

The first suppressed admission upgrades the registry to
`agent-session.coordination-registry.v3`, fencing older registry writers that
would drop the decision. Later writes keep v3 or a newer admitted fence even
after suppressed mail is removed. Readers still accept registry v1/v2; they cannot reconstruct decisions
an older writer already discarded. Before enabling the policy, upgrade all
registry writers and projection readers for that state directory. Rollback
requires a binary that understands v3; there is no automatic downgrade.

Suppression means stored without a wake; it is neither rejection nor loss, read,
acknowledgement, acceptance, or elevated authority.

A digest consumer uses authenticated `message inbox --state unread --category
progress --category report --format json` and the normal pagination cursor.
Inbox/show/wait and local send metadata expose the category, exact
`recipient_session_id`, `notification_suppressed`, and `created_at`, the existing
destination admission/storage timestamp. Remote delivery exposes the exact
recipient address and `persisted_at_epoch`. Suppressed messages remain readable
through `message show` and acknowledgeable through the unchanged revision CAS;
forwarding retains its original category and provenance while applying the new
destination's policy independently.

### Broker status

Broker projections use `agent-session.coordination-broker.v1` and expose only
state (`starting`, `ready`, `degraded`, `lost`, `stopped`), generation,
capability availability, heartbeat freshness, claim summary, and operation
summary, plus the additive [Operation diagnostics and reconciliation](coordination-runtime-v1.md#operation-diagnostics-and-reconciliation). They never expose a PID
as authority, a credential path, or a token.

`agent-session broker status --session ID --format json` places the projection
at `data` in `cli.agent-session.broker-status.v1`. Existing fields and schema
versions remain compatible. `data.session_incarnation` comes from the current
session record; a broker from a different incarnation is unavailable.
`--authenticated --capability-file FILE` retains the exact recipient capability
and fresh-heartbeat checks. Both modes read one locked registry snapshot and
never save normalization, renew a lease, acknowledge mail, claim a generation,
probe the prompt controller, submit a prompt, or wake a recipient.

`data.notification` is `null` when that incarnation has no receipt. Otherwise
it contains only this fixed set of metadata fields:

| Field | Meaning and source |
| --- | --- |
| `state` | Normalized receipt state: `queued`, `attempting`, `attempt_unknown`, `prompt_submitted`, or `undeliverable`. Submission is not consumption. Invalid stored states normalize to `undeliverable`. |
| `delivery_state` | `resolved` when no notification-eligible live unread mail remains, `submitted` when the generation has been notified and eligible mail remains, otherwise `queued`. Suppressed mail remains counted in `live_unread_count`. This diagnostic status does not replace the raw submission state. |
| `generation`, `attempted_generation`, `notified_generation` | Nonnegative receipt counters; an older attempt remains distinct from a newer queued generation. |
| `queued_at_epoch` | Current generation queue time, recovered when needed from canonical local unread admission metadata. Remote sender clocks are not used. |
| `attempted_queued_at_epoch` | Queue time retained for the attempted prompt, independent of newer mail or transcript compaction. |
| `attempted_at_epoch`, `updated_at_epoch`, `next_attempt_at_epoch` | Attempt, receipt update, and scheduled retry times from the normalized snapshot. Existing normalization may recover a missing update time or requeue a newer generation at observation time. |
| `last_reason` | Existing stable reason allowlist; unrecognized text becomes `notification-state-invalid`. An absent reason is `null`. |
| `live_unread_count` | Nonexpired canonical unread messages for the exact recipient incarnation. Remote senders and suppressed messages count; read, acknowledged, expired, and other-incarnation messages do not. |
| `resolved` | No notification-eligible live unread mail, or `generation <= notified_generation`. Delivery discovery and both delivery boundaries retire drained generations with reason `mailbox-drained`, preserving submission counters and timestamps, without changing mailbox records. Active `attempting` receipts retain their fence until an outcome is recorded. Resolution alone proves neither submission nor acknowledgement. |
| `controller_available` | `null`: status performs no controller probe, so availability is unknown. |
| `source` | `registry: "canonical-local"`; `normalization_applied` reports changes to this incarnation's in-memory receipt snapshot. `queued_at` is `stored-receipt`, `canonical-unread-admission`, or `unavailable`. |
| `observed_at_epoch` | Time the locked registry snapshot was observed. |

All epoch values are absolute Unix times in whole seconds. Receipt timestamps
with the historical zero sentinel, negative values, or values outside the supported
timestamp range project as `null`. Recovery follows the existing notification
normalization contract, including local forwarding/admission time; it changes
only the projected snapshot. Attempts keep their retained queue timestamp.
No raw stored reason, sender identity, message body, transcript, capability, or
private path appears in this bounded diagnostic object. Receipt inspection
makes reminder state observable; live wake and intermittent inbox acceptance
remain separate delivery evidence.

### Managed first-heartbeat readiness

Provisioning commits a broker reservation in `ready` state before releasing the
held launch. This reservation is not active presence or execution authority:
the exact incarnation, capability, generation, runtime identity, and fresh
identity-bound heartbeat remain required. The first beat and readiness check
read the private `broker-state.json` index, which contains broker selectors and
a completion-pending bit, without parsing retained mailbox data or acquiring
the registry flock.

The first-beat deadline remains two seconds, polled every
20 ms; uncertain initial storage is retried within that budget rather than
adding a two-second sleep. Existing pre-index state has a read-only atomic
registry snapshot fallback until a mutating registry owner installs the index.

The registry remains the writer and transaction owner. It publishes reductions
in heartbeat authority before registry commit and additions afterwards. Failed
publication therefore denies liveness rather than retaining revoked authority.

Completion work runs after heartbeat publication; an empty completion queue
skips the registry when its projection matches the committed registry file.
The index records that file's device, inode, size and modification time. A
registry-only atomic replacement by an older writer invalidates the projection,
so the next beat performs a locked queue observation and refreshes the file stamp
and pending bit
even when the queue is empty. An existing unstamped index similarly needs one
observation. These metadata probes never parse the registry or confer heartbeat
authority.

Read-only public snapshots open one validated
atomic registry inode without taking or creating the writer lock.
