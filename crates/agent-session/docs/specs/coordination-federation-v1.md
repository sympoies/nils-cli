# Coordination federation and service messages v1

Part of [Session Coordination V1](session-coordination-v1.md). Its scope and shared rules also apply here.

## Cross-host mailbox federation v1

Federation adds `message send --to-machine MACHINE`, `message peers --session ID`,
`message delivery --session ID --message ID`, and automatic remote `message reply`.
Delivery lookup checks the sender's federation outbox first, then resolves a
same-machine message from the local mailbox registry. Local results expose the
mailbox state, such as `unread`, `read`, or `acknowledged`.
`--host` continues to control attach-command generation. Omitted `--to-machine`
retains the local mailbox, and a `--to-machine` naming the sender's own machine
is delivered through the local mailbox without a federation journal entry. Bodies are cooperating-peer data that cannot grant new
authority; notification, inbox persistence, read/acknowledgement and accepting
work are separate events.

The source CLI reads the private `coordination/daemon-endpoint.json` in its state root and authenticates
to its own daemon with the current session capability. It never reads relay or machine operator secrets.
The daemon discovers authorized recipients at `GET /api/coordination/peers/v1` (query `source_session_id`
and `source_incarnation`) and submits to `POST /api/coordination/relay/v1` at the configured edge. Only
Console-registered sessions with exact ownership and current coordination support are eligible. The edge
owns owner isolation: same-principal messaging is the default, and cross-owner messaging requires an
explicit edge-configured allowance.

Discovery and delivery must apply the same allowance, retain both current ownership/incarnation checks,
and constrain each endpoint to its own owner's permitted machines. A mailbox allowance grants no board
visibility, Console list, attach, or transcript access. The daemon never accepts a caller-supplied
allowance or bypasses discovery for a new send. The edge binds its outbound service bearer to the source
machine and chooses destinations from its configured machine inventory. Configure an allowance in the
owning relay implementation; no additional daemon setting is required.

The JSON envelope is `agent-session.remote-message.v1`:

```json
{
  "schema_version": "agent-session.remote-message.v1",
  "message_id": "a UUID",
  "from": {"machine": "sympoies", "session_id": "source", "session_incarnation": "launch UUID"},
  "to": {"machine": "c8", "session_id": "target", "session_incarnation": "launch UUID"},
  "body": "peer message text",
  "body_sha256": "lowercase SHA-256 hex",
  "created_at_epoch": 1800000000,
  "expires_at_epoch": 1800086400,
  "reply_to": null,
  "reply_depth": 0
}
```

The destination `POST /coordination/messages/receive/v1` requires both the existing
operator bearer and a distinct `X-Agent-Session-Relay-Token`. It validates schema,
body digest, expiry, mailbox quotas, current recipient incarnation and ready
broker before atomically persisting inbox, origin, notification and dedup receipt.
It returns raw JSON `agent-session.remote-delivery.v1` with `message_id`,
`state: delivered`, `recipient` (the complete `to` address), and
`persisted_at_epoch`. `delivered` proves destination persistence only. Session
capabilities and bodies never appear in delivery status or diagnostic logs.

Peer discovery returns raw `agent-session.remote-peers.v1`, with `peers` containing
`machine`, `session_id`, `session_incarnation`, and `messaging_supported`.
The optional `messaging_unavailable_reason` string may be absent or null.
Recognized reasons are `daemon-unsupported`, `coordination-disabled`, and
`coordination-unavailable`; missing, null, or unrecognized reasons use the generic
unsupported observation without copying upstream text. `coordination-unavailable`
is transient rather than a definitive lack of messaging support.
`GET /sessions` advertises `data.coordination.remote_messaging_supported`;
per-session `coordination_mode` and `coordination.coordination_available` determine
current recipient readiness.

New submissions match the exact machine and session ID from authorized peer
discovery, preferring the first supported exact row when duplicate rows exist.
An absent exact address in a valid snapshot returns
`remote-recipient-not-discovered` with fixed guidance to check both address fields
using `message peers`. Absence may also reflect an omitted unavailable-host
snapshot and does not prove deletion or lack of capability. Failed or invalid
peer discovery retains `remote-messaging-unavailable`.

A discovered exact peer whose `messaging_supported` is not true returns
`remote-messaging-unsupported` for definitive or unknown unsupported reasons. An
explicit `coordination-unavailable` row receives one fresh discovery attempt bound
to the initially selected recipient incarnation. A persistent transient failure
returns `remote-messaging-unavailable`; no replacement incarnation is selected.
Neither refusal enqueues an envelope. Idempotent replay precedes discovery and
keeps the original recipient incarnation.

Source outbox submission returns a raw delivery projection (wrapped in the usual
CLI envelope for CLI callers): `message_id`, `state`, `sender`, `recipient`,
`attempts`, `reason`, and optional `receipt` and `refusal_details`.

The optional `refusal_details` object retains only quota refusal metadata: `quota`, `count`,
`limit`, `side` and `host`. Each field is optional; `count` and `limit` are
nonnegative JSON integers representable as `u64`, and each text field is bounded
to 128 UTF-8 bytes. Missing or invalid fields are omitted; if no supported field
is available, the object is omitted. Historical refusals without stored details
remain absent and are never reconstructed from current quotas. The object
survives terminal compaction and is also exposed by [Mail audit
v1](mail-audit-v1.md), including quota refusals conservatively classified as
`delivery-unknown`. Message bodies and upstream error text are excluded.

States are `queued`, `delivered`, `rejected`, or `delivery-unknown`. Network errors retry five seconds
after the request finishes until expiry. Due entries are selected by their retry deadline, so a
repeatedly timed-out entry cannot starve later messages. A retryable transport failure
(`remote-messaging-unavailable`, `coordination-unavailable`, including an unsupported refusal explicitly
classified as coordination unavailable) also defers every other already-attempted queued envelope for the
same destination address (machine, session and incarnation) to the same retry deadline.

An unavailable session or offline machine then costs one probe per destination session per retry interval
and cannot delay due envelopes for other destinations. A fresh envelope still gets its own first attempt.
The worker wakes on enqueue or the next pending deadline and sleeps indefinitely when no queued envelopes
or pending failure notices require work. No registry, journal or session lock spans network I/O. A source
restart preserves the original envelope and recipient incarnation. Retries with the same idempotency key
and content return the original identity without rediscovery; changed content is rejected.

Only queued (undelivered) envelopes carry bodies and count against the pending caps: 512 per destination
machine and 2048 in total. Once an entry is terminal (`delivered`, `rejected` or `delivery-unknown`)
every journal write compacts it into a body-free retained record (message ID, sender, recipient,
idempotency key, request digest, state, attempts, reason, expiry, and the receipt's `persisted_at_epoch`,
from which the accepted receipt is rebuilt). Replay, reply replay and delivery status read queued and
retained entries alike. Queued plus retained identities are bounded to 16384.

Journal admission reserves 5,242,880 bytes: the existing 262,144-byte bookkeeping reserve plus 4,980,736
bytes for bounded refusal diagnostics across all 2048 queued envelopes. The hard journal cap grows by
that same 4,980,736 bytes to 38,535,168 bytes, leaving the enqueue byte budget unchanged at 33,292,288
bytes. A submit whose encoded journal would exceed that enqueue budget is refused. Each bound refuses at
capacity and never evicts a live identity; identities are retained until 24 hours after expiry. Expiry
without a confirmed receipt remains unknown, including the case where the receiver saved the message but
its response was lost.

An admission rejection on the first attempt is `rejected`; after any prior unconfirmed attempt it is
conservatively `delivery-unknown`, retaining the last reason. In particular, replacement after a lost
response cannot prove nondelivery.

Terminal session-origin failures are reconciled into one local sender-mailbox
notice from the daemon's `agent-session-delivery` service origin. Its bounded
body contains the original message ID, a fixed reason code, delivery inspection
and retry guidance; it contains no original body, credential, or upstream error
text. The original sender incarnation remains the recipient: replacement never
retargets the notice. Existing mailbox admission and notifications apply. A
stable notice ID and retained ingress receipt prevent duplication after restart
or acknowledgement; unavailable notice persistence is retried without changing
the original terminal delivery state or replaying its remote envelope.

Destination receipts are retained until 24 hours after envelope expiry (maximum seven days),
up to 4096 receipts. At capacity the oldest receipts of acknowledged, expired,
or pruned messages are evicted first; capacity rejects instead of evicting the
deduplication ID of an unread or read message.
An exact retained receipt is returned before expiry, current recipient incarnation
or mailbox admission checks, without re-entering the inbox. Same-ID changed-content retries
are rejected. Remote senders retain machine/session/incarnation separately from
local sender identity; local controller guidance does not adopt remote origins.
Remote replies preserve the original sender incarnation and existing revision
and maximum-depth (16) checks.

Federation preserves the local coordination registry schema unless destination notification policy admits
suppressed mail; it never changes session runtime. Source envelopes live in the private
`coordination/federation-journal.json`, with supported schemas `agent-session.federation-journal.v1`
through v6. The v2 schema adds `retained` alongside `remote_outbox`, v3 retains category/forwarding
metadata, v4 retains audit timestamps, and v5 retains quota refusal details as described in [Mail audit
v1](mail-audit-v1.md). The v6 schema retains destination suppression receipts.

Reads and writes are bounded to 38,535,168 bytes, 2048 queued envelopes and 16384 identities, with the
admission reserve described above. A v1 journal is read and rewritten to a supported successor on the
next write. Stored refusal details select v5; once v5 is stored, subsequent writes keep at least v5 even
if those details are no longer present. Rollback requires a reader/writer that understands the stored
version and byte bound; releases that do not know v5 fail closed without discarding its metadata. There
is no automatic downgrade or reconstruction of missing historical details. Stored suppression receipts
select v6 and later writes keep v6, even after those receipts expire.

Rollback after that write requires a reader/writer that understands v6; a prior strict reader refuses the
successor schema rather than reading the new retained shape under an old version. A dedicated private
`coordination/federation-journal.lock` serializes journal reads/writes with the same bounded,
owner-checked file locking rules as the registry. Authorization uses session then registry then journal
lock order. Federation delivery and retry operations read the journal first; an
unmatched delivery ID is checked against the local mailbox registry so accepted
same-machine messages remain inspectable. The journal is saved atomically and no
lock spans HTTP. Unsupported or corrupt journal versions fail closed without
rewriting state.

Remote and local ingress share the existing mailbox admission rules, including
30 messages per pair per minute, a burst of 10 per second, the recipient mailbox
limits and the 68 MiB global body quota. Refusal returns typed `rate-limited` or
`quota-exceeded` before inbox persistence.

Every `quota-exceeded` error carries content-free `details`: `quota` (for example
`federation-pending-destination`, `federation-pending`, `federation-retained-ids`,
`federation-journal-bytes`, `recipient-messages`, `recipient-bytes`, `registry-message-bytes`,
`registry-bytes`, `cursors`, `recipient-cursors`, `receipts`, `principal-receipts`,
`principal-receipt-bytes`, `completion-events`, `claim-fence-files`, `claim-fence-operation-files`,
`provider-turn-receipts`), the current `count`, the `limit`, the evaluating `host`, and `side`: `source`
for the sending host's federation outbox, `destination` for remote admission, otherwise `local`.

`count` is the value that admitting the request would reach. Source federation refusals add
`destination_machine`, `pending`, `pending_to_destination`, `delivered` and `retained`. The message ends
with `(<quota> <count>/<limit>)`. The CLI reports its local daemon's refusals with that daemon's own
code, message and details; only refusals that came back through the relay are described as "remote
mailbox request was rejected", keeping just these quota fields from the relayed details.

Recipient quota warnings are derived without changing message retention or
admission. `message inbox` adds nullable `mailbox_quota_warning`, independent
of the page/state filter; public list/glance metadata exposes the same warning
when present. At 80 percent of either the 256-message or 4 MiB recipient limit,
`status` is `near-limit`; at either limit it is `at-limit`. The warning includes
`message_count`, `message_limit`, `byte_count`, `byte_limit`, and `recovery`.
Counts use the existing admission predicate across the recipient's messages:
acknowledged, expired and deleted messages remain excluded. Warning projection
does not delete or acknowledge evidence.

For `recipient-messages` and `recipient-bytes`, `quota-exceeded` additionally
names the read/ACK recovery in its message and includes the same fixed
`recovery` object (`actor: recipient`, `action: read_and_ack`, `inbox_unread`,
`inbox_read`, `show`, `ack`, `guidance`). Relay receivers derive this instruction
locally from the allowlisted quota name; they never copy remote recovery text
or body data. Other quota axes retain their existing diagnostics.

The authenticated recipient lists unread/read mail with `message inbox`, uses
`message show` to read it, and ACKs handled messages with `message ack` using
the returned revision and a unique idempotency key. Show can advance revision;
ACK must use the latest one. Reading alone does not release quota. Operators
and senders ask the recipient to do this; they cannot ACK another recipient's
mail. Keep unread and archived evidence. The guidance does not promise that
ACKing current-incarnation mail resolves every historical-incarnation quota.

Destination inbox, notification and deduplication receipt share one existing
registry commit. Authoritative sender identity is the existing
`sender_session_id` string: `remote:` followed by a canonical JSON tuple of
machine and session ID. Colon is forbidden by the local session ID validator, so
this representation cannot collide with a local session or controller. The
existing `sender_incarnation` retains the foreign incarnation. New projections
and replies decode the tuple; older writers preserve these existing fields and
cannot reply to it as a local session. No remote origin is normalized into a
local sender. Malformed reserved identities never route locally.

Receive receipts use the existing receipt schema and quota, with reserved
principal `remote:receive`, incarnation `v1`, operation
`remote-message-receive`, and the globally unique message UUID as key. The
receipt digest covers the full envelope and its TTL is extended through envelope
expiry plus 24 hours. Old receipt sweepers preserve this exact expiry field.
Same-ID local-message collisions also reject before mutation. This keeps inbox,
origin, deduplication and notification atomic without a two-file recovery gap.

Rollback disables federation values and retains the journal. Older local writers
continue to operate unchanged registry schemas; pending remote deliveries pause
until a federation-capable daemon resumes. Existing sessions and pinned broker
and hook helpers do not require recreation. Never delete the journal to roll back
or reinterpret an opaque remote sender as a local session.

## Service-origin submission

`message service-send --service ID --service-generation GENERATION
--credential-file FILE --to ID --body-file FILE --idempotency-key KEY
[--to-machine MACHINE] [--expires-in DURATION]
[--expected-recipient-incarnation INCARNATION]` submits through the owning
supervised daemon, without managed-session authentication. Its CLI envelope is
`cli.agent-session.message-service-send.v1`. Admission/configuration/revocation
are separate operator controls, described in the
[daemon runbook](../runbooks/serve-daemon.md#admit-a-local-mailbox-service).

`POST /coordination/services/messages/v1` accepts a direct loopback peer only,
with its distinct admitted service token in `Authorization: Bearer`. Unknown
fields and proxied requests are rejected. The JSON request is:

```json
{
  "service_id": "reporter",
  "service_generation": "generation-1",
  "to_machine": null,
  "to_session": "recipient",
  "body": "service-authored private text",
  "idempotency_key": "operation-1",
  "expires_in": "24h",
  "expected_recipient_incarnation": null
}
```

The daemon derives the source machine from its own configuration, authenticates
service ID/generation and credential possession, and revalidates admission at
commit. Neither a body-file CLI selector nor a session capability grants service
authority. Operator/federation tokens cannot authenticate this route. Existing
managed-session send authentication and wire serialization are unchanged.
An optional expected recipient incarnation rejects stale selectors; all
submissions require a full session ID and bind the exact live ready recipient
before commit; abbreviated IDs never select a service recipient. Receipts replay
before fresh discovery, preserving the original destination across movement.
Local and remote keys cannot silently cross routing channels.

Local mailbox responses retain `agent-session.message.v1`, with the sender
projection `{kind: "service", machine, service_id, service_generation,
authenticated: true}` and no session ID/incarnation fields. Recipient show/wait
bodies are `untrusted_service_data`. Internal sender storage uses `service:`
plus the canonical JSON tuple `[machine, service_id]`, with generation stored
in the existing sender-generation field. This opaque principal is never loaded
as a session or adopted as controller guidance. The existing mailbox lock,
quota admission, notification scheduler and receipt store own local persistence;
there is no separate service queue. Service replies are explicitly unsupported
(`mailbox-service-reply-unsupported`).

Remote submission uses the existing private federation journal and transport.
Discovery at `GET /api/coordination/peers/v1` supplies `source_service_id` and
`source_service_generation` instead of session selectors. The relay must bind
those selectors to the authenticated machine and its explicit service admission
before returning authorized recipients or forwarding an envelope. The distinct
wire schema is `agent-session.remote-service-message.v1`:

```json
{
  "schema_version": "agent-session.remote-service-message.v1",
  "message_id": "a UUID",
  "from": {
    "machine": "source-machine",
    "service_id": "reporter",
    "service_generation": "generation-1"
  },
  "to": {
    "machine": "destination-machine",
    "session_id": "recipient",
    "session_incarnation": "launch UUID"
  },
  "body": "service-authored private text",
  "body_sha256": "lowercase SHA-256 hex",
  "created_at_epoch": 1800000000,
  "expires_at_epoch": 1800086400,
  "reply_to": null,
  "reply_depth": 0
}
```

Both origin shapes reject unknown fields. The envelope schema discriminates
session from service origin; hybrid identities, schema/origin mismatches,
service reply parents and nonzero reply depth are invalid. Service tokens never
cross the machine boundary. Destination ingress still requires the existing
operator bearer plus distinct relay ingress token; that trusted relay attests
to the admitted source tuple. A caller's unauthenticated source strings never
replace those credentials. Destination body/digest/expiry/recipient/quota checks
and atomic mailbox/dedup persistence use the existing receive transaction.

Delivery projections remain `agent-session.remote-delivery.v1`, with a service sender including `kind:
"service"`; delivery receipts remain unchanged. Network errors retain queued envelopes and retry through
the existing drain. Rejected or uncertain service deliveries retain their original envelopes inside the
same bounded journal until message expiry, allowing safe operator reconciliation without retargeting or
losing live queued content. The existing drain deadline wakes for that expiry even without another
submission; bodies then compact to the existing body-free identity retention. These held failed envelopes
consume the existing outbox count and byte budgets.

Delivered service records and managed-session terminal records compact as before. An uncertain prior
attempt remains `delivery-unknown` if a later rejection cannot prove nondelivery. Revocation blocks
further source submission/replay but preserves already committed deliveries. Rollback retains the private
journal: older daemons may refuse a service-bearing journal, so pause remote submissions and restore a
compatible owner rather than rewriting or deleting pending data. Local managed mailbox operations
continue to use their existing registry independently.

Service diagnostics contain fixed messages/codes and typed `retryable`,
`next_action` and bounded `recovery` fields. Body text, credential values, IO
paths and arbitrary peer error details are excluded. Typical codes are
`mailbox-service-unauthorized`, `mailbox-service-forbidden`,
`mailbox-service-request-invalid`, `mailbox-service-unavailable`, the existing
mailbox body/expiry/quota/rate/idempotency codes, and
`session-incarnation-conflict`. HTTP uses 401 for service authentication denial,
403 for nonlocal/proxied submission, 409 for recipient/idempotency revision
conflicts, 400/422 for invalid requests, and 503 for unavailable coordination or
transport. The CLI uses usage/data/unavailable exit categories (64/65/69).

An absent remote address in a valid discovery snapshot returns
`remote-recipient-not-discovered` (HTTP 422 / CLI 65) with the fixed address-check
guidance, `retryable: false`, `next_action: inspect_submission_contract`, and
operator recovery. Inspect discovery and correct the address before a new
submission. Failed discovery and unavailable transport remain retryable with
`retry_same_submission` and `same_idempotency_key` recovery.

## Message categories and recipient forwarding

Categories are sender-declared routing metadata: `uncategorized`, `progress`,
`handoff`, `blocker`, `decision`, and `report`. They confer no work authority.
`message send`, `reply`, and `service-send` accept `--category`; omission
projects as `uncategorized`. Replies declare their own category. Stored v1
messages add optional `category` and `forwarding` fields with defaults; existing
message and inbox projections keep their schema names and add the effective
category plus optional provenance. Inbox remains body-free. The local HTTP
send/reply bodies accept optional `category`; inbox HTTP queries accept one
`category`. The CLI's repeatable categories are an OR filter, composed with
`--state`, applied before pagination and normalized into the opaque cursor.
Changing the filter while reusing a cursor returns `cursor-invalid`.

```sh
agent-session message send --from "$AGENT_SESSION_ID" --to <recipient> \
  --category progress --body-file progress.txt --idempotency-key progress-001
agent-session message inbox --session "$AGENT_SESSION_ID" --state unread \
  --category progress --category handoff --format json
agent-session message forward --session "$AGENT_SESSION_ID" --message <message-id> \
  --if-revision 1 --to <destination> --to-machine <configured-machine> \
  --category progress --category handoff --idempotency-key route-source-rule-001
```

Forward selects one exact received message. Its repeated `--category` flags are
an optional source guard; it cannot change the category. The current recipient's
capability and incarnation authorize the operation. The implementation verifies
the source revision, category, body and expiry under the final commit lock,
creates a fresh destination message with identical body/category and capped
expiry, persists an idempotent source receipt (local) or durable outbox identity
(remote), and schedules the destination's usual body-free reminder. It never
reads, acknowledges or advances the source revision. Expired/quarantined sources
cannot be forwarded. `message-category-conflict`, `message-revision-conflict`,
`message-forward-loop`, `message-forward-depth-exceeded` and
`message-forward-invalid` are content-free errors.

The immediate `sender` remains the authenticated forwarder. Optional `forwarding` provenance is
explicitly attested by the forwarder, with `attestation: "forwarder"`, root `original_message_id`,
`original_sender` (session address or service origin), `original_recipient`, original creation and expiry
epochs, body SHA-256, and `hops`. Each hop records `source_message_id`, `source_revision`, `forwarder`,
`recipient` and `forwarded_at_epoch`. Original identity is provenance, not fresh end-to-end sender
authentication. All forwarded content remains untrusted peer/service data; a forwarded service body stays
`untrusted_service_data`.

Remote ingress retains the authenticated envelope creation epoch separately from local inbox
creation/ingress timestamps; a new forwarding root uses that source epoch. Existing stored remote
messages without this optional clock retain their previously stored creation value. Replies target the
immediate forwarder. Controller resume's existing `forwarded_from_incarnation`/`forwarded_at_epoch`
fields remain separate.

Forwarding rejects a destination matching the root sender/recipient or a prior hop participant by
machine/session identity, including a replaced incarnation. Eight hops are the maximum. Supported local
controller-authorized guidance carry records optional `recipient_transfers` on the existing hop, leaving
the historical recipient unchanged. Each event retains the received copy's UUID, exact `from`/`to`
addresses, controller address, source revision and transfer epoch. It changes only the incarnation of the
same machine/session.

The controller must equal that hop's forwarder, or the transfer's own `from` address for a same-session
resume carry, and share the recipient machine; creation occurs under the existing broker check and
controller authorization guard. Eight total transfers are allowed across all hops. Omitted or empty
arrays mean no transfers; null and unknown event fields are rejected.

Transfers continue exact hop identity and nondecreasing timestamps. Repeated transfers bind the same copy
and strictly increasing revisions; the next hop binds that copy UUID, a newer revision and the exact
transferred incarnation. The old carry metadata alone cannot authorize a transition. Transfers are
historical forwarder-attested data and confer no new authority. Incoming transport requires an empty
final-hop transfer array, preserving exact immediate sender/destination binding; only the local receiver
can add an authorized carry before its next forwarding operation. Ingress checks chain continuity, origin
kind, destination, body digest, expiry and bounds.

Identical operation retries replay before looking up the source or discovering the destination; changed
source/revision/destination/category-guard inputs conflict. A durable source receipt and destination
provenance provide the audit trail. Remote terminal compaction retains category/provenance without
retaining the body.

The recipient acknowledges its original independently from the destination's
copy. A routing application should save its compact digest and confirm remote
`delivery.state == "delivered"` before acknowledging the original. `queued`,
`rejected` or `delivery-unknown` is insufficient. A delivery receipt means saved,
never read or accepted; an acknowledgement means mailbox handling, never work
acceptance. Applications own category-to-destination configuration, stable
per-source/per-rule replay keys, retry/status polling, compact digests and
acknowledgement ordering. Rules should exclude forwarded messages by default.
A missing destination or failed/unknown delivery leaves the source available.

### Mixed-version rollout

Untagged remote requests/envelopes omit the new fields and preserve v1 wire
shape and replay digests. Tagged/forwarded session transport uses
`agent-session.remote-message.v2`; tagged service transport uses
`agent-session.remote-service-message.v2`. Upgraded ingress accepts exact v1
without extensions and v2 with category/provenance; it rejects version/field
mismatches. Existing authenticated submission/relay/ingress routes are retained.
Extended outbox records use `agent-session.federation-journal.v3`, retaining
body-free category/provenance data. New source timestamp metadata uses journal
v4; existing v1/v2/v3 remain readable. See [Mail audit v1](mail-audit-v1.md)
for read-only owner projections and timestamp compatibility.

Old tolerant projection readers can still read additive JSON; old stored
messages project as uncategorized. Old strict remote endpoints or validating
relays reject v2 visibly; no downgrade strips category/provenance. Upgrade
source and destination daemons and schema-validating relays before enabling
routing. Do not let an older registry writer rewrite new metadata: an older
binary does not preserve fields it does not know. Untagged v1 operation remains
available during rollout. Release and routing activation belong to the
coordinator application owner.

The headless HTTP fixture accepts `--previous-policy-agent-session-bin <path>`
for a federation-aware build from before recipient notification policy. It
checks that the old registry writer and daemon journal reader refuse the
successor formats without rewriting bytes, then verifies new readers preserve
suppression. The separate `--previous-agent-session-bin` option retains the
older non-federated facade compatibility checks.
