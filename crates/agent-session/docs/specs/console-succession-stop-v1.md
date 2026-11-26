# Console succession stop v1

This contract defines a fenced stop of the predecessor in a regular
`agent-session start` succession under one Console owner.
[#4100](https://github.com/sympoies/nils-cli/issues/4100) freezes the
contract before the relay and daemon implementations. Manual adoption
and provider-conversation moves are outside v1.

The successor may take over only after it verifies the terminal stop receipt
for its succession. The operation retains the predecessor record and its
history. Subsequent removal uses the native conditional delete with the same
expected incarnation, stopped proof and unread-empty predicate.

## Routes and media type

The source daemon authenticates the successor. The destination daemon
controls the predecessor, even when both roles run on the same machine.

| Hop | Stop POST | Receipt GET | Authority |
| --- | --- | --- | --- |
| Managed successor to source daemon | `/sessions/{caller}/console-succession-stop/v1` | `/sessions/{caller}/console-succession-stop/v1/{operation_id}` | The exact current local session capability in `Authorization: Bearer`. |
| Source daemon to Console relay | `/api/coordination/sessions/{machine}/{id}/stop/v1` | `/api/coordination/sessions/{machine}/{id}/stop/v1/{operation_id}` | Configured origin bearer and source-daemon caller attestation. |
| Console relay to destination daemon | `/sessions/{id}/succession-stop/v1` | `/sessions/{id}/succession-stop/v1/{operation_id}` | Destination machine bearer, relay ingress token and succession-stop binding. |

Here `{caller}` is the successor's exact managed session ID. `{machine}`
and `{id}` are the predecessor daemon and exact managed ID. Bind them to a
configured route and verified session identity. Prefix lookup is forbidden.

POST sends a UTF-8 JSON object with exactly one `Content-Type` header whose
value is `application/vnd.agent-session.console-succession-stop.v1`. The
literal has no parameters or suffix. The scoped GET sends no body. Replies
on the v1 routes use that same media type and `Cache-Control: no-store`.
Reject duplicate or unknown JSON members, nulls and unexpected query keys.
Protect the transport on both relay hops, pin each configured peer and
disable redirects. Keep credentials, attestations and raw diagnostics out
of logs, browser responses and runtime environments.

The POST body requires the following members. The source also requires
`machine` and `session_id` as nonempty strings for the relay path. It
consumes those two selectors and serializes the remaining members once.
The relay forwards those exact body bytes to the destination.

| Key | Value |
| --- | --- |
| `schema_version` | Literal `agent-session.console-succession-stop.v1`. |
| `operation_id` | Canonical UUIDv7 minted by the successor before submission and reused for the same attempt. |
| `expected_session_incarnation` | Canonical non-nil UUID of the predecessor's runtime launch. |

Canonical UUID text is lowercase and hyphenated. The source GET requires
exactly three query members, `machine`, `session_id` and
`expected_session_incarnation`, matching the remembered submission. The
relay and destination GET each require only
`expected_session_incarnation` in the query. The operation ID comes from the
path. All hops validate the exact selectors. A receipt GET cannot start
an operation. Missing v1 support or an unsupported media type fails closed
at the corresponding hop. The client never substitutes interrupt,
archive, ordinary delete or a direct stop.

## Delegation and receiver proof

The coordinating service verifies the acknowledgement for the regular
succession before issuing the `succession_stop` delegation. It records the
acknowledged succession and binds the grant to the exact successor
machine, managed ID and incarnation, the exact predecessor machine,
managed ID and incarnation, and the Console owner shared by both. The
successor and predecessor must be distinct sessions. The service is the
issuer and revocation authority. A role, lineage, asserted owner, operator
bearer or [worker delegation](scoped-worker-delegation-v1.md) cannot
substitute for this grant.

The source validates the successor's capability and current runtime
incarnation on each POST and GET. It derives the caller from that
authenticated record and sets exactly one
`X-Agent-Session-Succession-Caller` header on the relay request. The value
is unpadded base64url of a UTF-8 JSON object with exactly the required
string members `session_id` and `session_incarnation`. The relay derives
the caller's machine from the origin bearer. The source accepts no caller
attestation or relay credential from the requester.

On every POST and GET the relay checks the attested current caller, its
machine access, same-owner grants for both exact identities and the
matching active service delegation. It derives the owner solely from its
authoritative grant store. After verified predecessor removal, only the
retained original exact ownership binding may stand in for the removed
predecessor, and only for receipt access. A replacement incarnation or a
new owner cannot claim the receipt. Perform these checks before disclosing
even the presence of an operation and recheck before issuing any downstream
request or returning a retained receipt. Unavailable authorization state
fails closed.

The destination uses the [relay ingress credential boundary](console-resume-move-transport-v1.md#delegated-caller-binding).
The relay sends one `Authorization: Bearer`, one
`X-Agent-Session-Relay-Token` and one
`X-Agent-Session-Succession-Stop-Binding`. Only the authenticated relay
constructs the binding. Its value is unpadded base64url of a UTF-8 JSON
object, at most 2 KiB encoded, with these required non-null string members.

| Key | Value |
| --- | --- |
| `schema_version` | Literal `agent-session.console-succession-stop-binding.v1`. |
| `scope` | `stop` for the destination POST or `receipt` for the destination GET. |
| `delegation_id` | Exact canonical non-nil UUID of the service grant verified for this request. |
| `owner` | Exact Console owner from the grant store. |
| `caller_machine`, `caller_session_id`, `caller_session_incarnation` | Exact source-attested successor. |
| `machine`, `session_id`, `session_incarnation` | Exact pinned predecessor. |
| `operation_id` | Exact operation UUIDv7. |
| `retry_until_epoch` | The bound retry deadline as a base-10 UTC Unix second string. |
| `body_sha256` | The lowercase hexadecimal SHA-256 of the received body bytes. For GET, hash zero bytes. |

Authenticate both destination credentials before parsing selectors or
disclosing state. Reject repeated headers and invalid encoding, schema,
members or identities. Match the scope to the method and route, the
predecessor machine to the local configured identity, the session to the
path, the incarnation and operation to the request, and the digest to the
exact body bytes. Match any stored operation against the entire authorized
binding, excluding per-request scope and body digest. The destination trusts
the authenticated relay for a fresh grant check on each dispatch. Grant
revocation bars subsequent dispatch and disclosure. A request admitted before
revocation can settle that exact operation and leaves its fence intact.

The successor proves receipt access by authenticating again as that exact
current incarnation through its source daemon. Possession of an operation
ID, shared owner membership or a copied receipt does not prove access.
The source accepts a receipt only from its configured relay after matching
the receipt's successor, predecessor and operation to the authenticated
caller and the requested succession.

## Console and native admission

1. The relay serializes succession stop with writes and lifecycle changes
   for the predecessor. In one durable transaction, scan pending provider
   input for the exact predecessor across all principals and claim the
   lifecycle slot and operation if clear. Input that can still reach the
   provider includes queued native handoffs, sending requests and unknown
   outcomes. If admission wins first, stop sees the pending write. If
   stop wins first, later input enqueue, claim and retry are blocked for the
   predecessor. The fence survives a relay restart and applies to every
   Console admission path for that incarnation.
2. The destination acquires the normal session-record lock for the exact
   managed ID. Under that lock, reload the record, recheck the launch UUID
   and retained runtime identity, check that the unread mailbox is empty
   and run the native guarded-stop admission checks. Use the authoritative
   coordination registry read with the record lock held. Local and remote
   delivery must serialize with it. Fail closed if the predicate cannot
   be proved. A list projection or notification count cannot supply the
   predicate, and the route has no unread override.
3. Under the record lock, persist the exact operation and an
   incarnation-bound native admission fence before sending the guarded
   provider exit. The fence must serialize with competing native input and
   lifecycle writers, including mailbox admission, after the lock is
   released. Preserve already admitted input and reject stop when it could
   still land. Use the native idle, empty-composer and exact process guards
   and supported graceful exit used by
   [source stop and retain](resume-move-proof-v1.md). Release the record,
   activity and provider locks while exit hooks finish. An in-process
   restart guard alone cannot supply the durable succession fence.
4. Reacquire the record lock and revalidate the exact incarnation before
   committing a terminal receipt. Prove that the native runtime and its
   recorded process boundary are stopped and earlier admitted writes
   cannot still commit. A disappeared pane, closed connection or elapsed
   timeout cannot prove this. If the exact runtime is already verified
   stopped at admission, perform the same unread check and persist the
   fence and terminal receipt without sending exit. If outcome persistence
   or stop verification is uncertain, retain the fences and reconcile the
   exact operation. Never force stop through this route.

The relay verifies the destination receipt and durably converts its
in-flight lifecycle claim to an admission block for the exact old
incarnation before projecting a terminal outcome. The block rejects old
writes regardless of the principal and does not authorize input to a new
incarnation. Keep it until authoritative evidence rules out every further
old write, including delayed or previously admitted work. Receipt expiry,
grant revocation, a missing record by itself or a daemon restart cannot
release it. Archive, delete and restart must contend with an unresolved
stop and may not clear the block. A definitive pre-effect refusal can
release only its own provisional claim once no delayed stop can act.

## Receipt and replay

On first authenticated admission, the relay issues an operation entry and
sets its absolute retry deadline to its UTC issuance time plus 24 hours.
One day covers retries across a restart or brief outage. The relay and
destination both persist the same deadline before allowing an effect.
Each hop uses the recorded Unix second as given and refuses a first
effect when its own UTC clock reaches it. A clock ahead of the issuer
may deny early. A clock behind the issuer still denies when its own
clock reaches the recorded deadline. V1 assumes synchronized UTC
clocks and gives no skew allowance. No hop recomputes or extends it.
The deadline does not prevent reading an authorized retained receipt.

An operation ID binds the complete authorized tuple, including the
delegation and both incarnations. Use validated field values to compare
requests, so JSON object ordering is immaterial across retries. The relay
and destination claim the key durably before dispatch or native effect.
An exact repeat returns the original terminal response or reconciles the
same operation without another exit. A changed binding under the same ID conflicts.
A different ID cannot bypass an unresolved operation or a retained
admission block. Once a definitive pre-effect refusal is recorded, the
successor can submit a new operation after the cause clears and the relay
verifies authorization again.

Use the same ID on transport loss and read back that operation. A missing
destination receipt after possible dispatch, malformed reply or timeout
is unresolved. Read the native outcome from the pinned destination.
Only the native durable operation entry and evidence against the
original exact runtime can settle a dispatched stop.

Before the deadline the relay may retransmit the exact request with the
original binding and fence if a native read establishes that no effect
was admitted and native deduplication is available. A late copy contends
for the same native entry before any effect. When the native
entry shows that exit might have been submitted, reconcile from proof
without submitting it again. Every reconciliation GET is read-only and
never invokes stop or delete.

All replies have exactly `schema_version` (the request schema literal),
`ok`, and either `data` or `error`. HTTP 200 carries `ok: true` and the
operation projection in `data`. A verified POST that exits the predecessor
returns the terminal projection. An authenticated GET can also return a
retained pending, refused or unresolved projection. The projection has
these exact members.

| Member | Value |
| --- | --- |
| `operation_id` | The exact operation UUIDv7. |
| `caller`, `predecessor` | Objects with exactly `machine`, `session_id` and `session_incarnation` for the successor and predecessor respectively. |
| `state` | `pending`, `unknown`, `refused` or `stopped`. |
| `stop_result` | `exited` or `already_stopped` for `stopped`; null otherwise. |
| `code` | The fixed refusal code for a recorded `refused` outcome; null otherwise. |
| `retry_until_epoch` | The original retry deadline as an integer UTC Unix second. |

Only the destination creates the immutable `stopped` receipt. The relay
and source verify the complete returned operation and identity binding
and the permitted state and result before returning it. HTTP 409
`session-already-stopped` includes that same terminal receipt in
`error.details.receipt`. This proves the exact stopped predecessor
without taking a stop action. Both forms of the receipt require the relay
admission block before delivery. They authorize the successor to continue its existing
retirement flow, which rechecks native stopped and unread state at delete.

The coordinating service revokes the grant after verified receipt
read-back by the successor. It may revoke early to withdraw authority.
Both forms of revocation end submissions and receipt access immediately.
They preserve any admitted stop, its saved evidence and its fence. The
relay must check current authority before projecting a result after an
in-flight request.

Retain terminal receipts outside the predecessor record in both stores
until both the retry deadline has passed and the coordinating service has
verified deletion of the exact predecessor by reading back authoritative
absence after conditional deletion. A different incarnation or missing
entry in an open list cannot establish that read-back. Each store must
have verified deletion read-back evidence and establish that the deadline
has passed in issuer UTC before compaction. A fast local clock alone
cannot shorten retention. If either check is unknown, retain the receipt.

Pending and unknown operations remain unresolved across the deadline. Compaction must preserve
the binding or a durable replay barrier for as long as the ID could
otherwise be admitted again. Receipt retention provides no authority to
read a revoked grant, and receipt compaction cannot remove an admission
block.

## Refusals

After authenticating the hop, validate the POST media type and request
shape, verify delegation and ownership, then inspect the operation and
target. Authentication and authorization refusals disclose no
operation or target state. Errors use `ok: false` and `error.code` only
except for the verified already-stopped receipt defined above.

| HTTP | Code | Condition |
| --- | --- | --- |
| 400 | `invalid-request` | Invalid selectors, version, headers other than the credentials or binding, or JSON shape. |
| 401 | `unauthorized` | Missing, invalid or repeated credentials for the hop. |
| 403 | `succession-stop-delegation-missing` | No service-issued grant authorizes this succession. |
| 403 | `succession-stop-delegation-revoked` | The matching grant was revoked. |
| 403 | `succession-stop-delegation-mismatch` | Unavailable caller incarnation, forbidden route, or absent, invalid or mismatched caller attestation or destination binding. |
| 403 | `succession-stop-owner-mismatch` | The two exact session identities do not share an authorized owner. |
| 404 | `stop-operation-not-found` | No receipt exists for this authorized lookup; absence cannot settle an earlier dispatch. |
| 409 | `session-incarnation-conflict` | The exact predecessor record is absent or its current incarnation differs during stop admission. |
| 409 | `session-mailbox-not-empty` | The native unread predicate is false. |
| 409 | `session-already-stopped` | The native proof verifies that the exact predecessor was already stopped. |
| 409 | `session-stop-busy` | Native state or another lifecycle operation prevents guarded stop. |
| 409 | `outbox-pending-replies` | Pending Console input could still reach the predecessor. |
| 409 | `outbox-lifecycle-pending` | Another operation owns the relay lifecycle slot or admission block. |
| 409 | `operation-id-conflict` | An operation ID already binds different authorized values. |
| 409 | `stop-operation-expired` | An operation reached its recorded deadline before native admission. |
| 415 | `unsupported-media-type` | POST has a missing, repeated or incorrect Content-Type. |
| 503 | `session-mailbox-unknown` | Native unread state could not be verified. |
| 503 | `succession-stop-unavailable` | The v1 path, authentication state, or durable pre-dispatch admission is unavailable. |
| 503 | `session-stop-outcome-unknown` | After possible dispatch, the exact stop outcome could not be verified, recorded or projected. |

Preserve the verified fixed code and HTTP status across the hops. An
untyped or malformed downstream result after possible dispatch is unknown.
Never synthesize a receipt from HTTP status, mailbox contents, list
output or an upstream error string.

## Freeze and implementation acceptance

The spec PR requires designated approval, security review of the
delegation and fence, the consuming Console owner's sign-off at the exact
head, and the required CI checks. The merged spec is the implementation
reference.

- Verify that a same-owner successor reaches only its acknowledged
  predecessor. Refuse wrong incarnations, missing, revoked and mismatched
  grants, different owners, and receipt access by any other incarnation.
- Race stop against input admission and retry from multiple Console
  principals and against local and remote mailbox delivery. If input
  wins, require a refusal before exit. If stop wins, require rejection of
  late writes and protect concurrent lifecycle operations. Exercise unread
  and unavailable mailbox state under the native record lock.
- Exercise already-stopped proof, exact replay, lost replies, relay or
  daemon restart, later incarnations, and deletion read-back on both sides
  of the deadline. Verify one native exit effect and retention of the
  admission block whenever old writes are still possible.
- Verify that unsupported media or unavailable v1 routes cannot reach a
  different stop path. Complete one scheduled live regular succession
  drill after implementation. A dry run cannot satisfy that acceptance.
