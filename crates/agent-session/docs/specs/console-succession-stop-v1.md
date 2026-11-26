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
| `operation_id` | Canonical UUIDv7 bound by the coordinating service at grant issuance and reused for the same attempt. |
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

### Issuance and revocation on the relay

The coordinating service is the operator-configured service that owns the
regular succession and verifies its acknowledgement. It must verify and
durably record the ACK for the two exact runtimes before issuing the grant.
The relay authenticates this service with its own configured service bearer
in `Authorization: Bearer`. The credential is scoped to the two grant
routes below and distinct from origin, machine, relay ingress and
attested-create credentials. Refuse any other caller before parsing grant
selectors. The source daemon attests calls from managed successors. It
never issues or revokes grants.

| Method and relay path | Action |
| --- | --- |
| `POST /api/coordination/succession-stop-delegations/v1` | Issue and durably register the exact grant after the service has verified the ACK. |
| `DELETE /api/coordination/succession-stop-delegations/v1/{delegation_id}` | Revoke the exact grant while retaining its record. No query or body. |

Issue uses the v1 media type and accepts a JSON object with exactly
`schema_version: "agent-session.console-succession-stop-delegation.v1"`,
`delegation_id`, `operation_id`, `caller` and `predecessor`. The service
mints the canonical non-nil UUID `delegation_id` once as the issue
idempotency key and binds the canonical UUIDv7 of the stop operation. It
gives that operation ID to the exact acknowledged successor. The
`caller` and `predecessor` objects each require exactly `machine`,
`session_id` and `session_incarnation`, all non-null strings in canonical
form. The two must identify distinct sessions. The relay validates the
configured machines, exact ownership grants, caller machine access and
shared Console owner. The service supplies no owner member.

The relay has one durable grant registry. Each record has the four
request members `delegation_id`, `operation_id`, `caller` and
`predecessor`, plus the following fields. The relay derives the owner and
sets the timestamps and deadline at issuance.

| Field | Value |
| --- | --- |
| `scope` | Literal `succession_stop`. |
| `owner` | The exact shared Console owner. |
| `state` | `active` or `revoked`. |
| `issued_at_epoch` | Integer relay UTC Unix second of first issuance. |
| `retry_until_epoch` | Integer Unix second of the original issuance plus 24 hours. |
| `revoked_at_epoch` | Integer relay UTC Unix second of first revocation, or null while active. |

Issue and revoke return HTTP 200 with the grant schema literal above as
`schema_version`, `ok: true` and the exact stored record in `data`. A
repeat with the same ID and validated body returns that record without
mutation, including after revocation. A different body under the same
ID or reuse of an operation ID by another grant returns
`operation-id-conflict`. Repeated revoke returns the original revoked
record and revocation time. An unknown revoke ID returns
`succession-stop-delegation-missing`. An issue for a tuple with another
active grant returns `succession-stop-delegation-mismatch`. A revoked
ID cannot reactivate. Registry errors fail
closed. Serialize grant revocation with relay stop admission and
authorization for each outbound POST.

### Delegated successor calls

The relay looks up the grant by both exact machine, session ID and
incarnation tuples under the shared owner. It requires one active match
bound to the requested operation ID. A different operation ID or
multiple active matches return `succession-stop-delegation-mismatch`. When there is no active match,
return `succession-stop-delegation-revoked` for a retained revoked match
or `succession-stop-delegation-missing` when no match exists. Retain
revoked records in v1. The source sends no grant token or delegation ID.
A role, lineage, asserted owner, operator bearer or
[worker delegation](scoped-worker-delegation-v1.md) supplies no grant.

The source validates the successor's capability and current runtime
incarnation on each POST and GET. It derives the caller from that
authenticated record and sets exactly one
`X-Agent-Session-Succession-Caller` header on the relay request. The value
is unpadded base64url of a UTF-8 JSON object with exactly the required
string members `session_id` and `session_incarnation`. The relay derives
the caller's machine from the origin bearer. The source accepts no caller
attestation or relay credential from the requester.

On every caller POST and GET the relay matches the attested caller
incarnation to the caller's current Console grant. A missing or different
current incarnation is `succession-stop-delegation-mismatch`. The relay
checks machine access, same-owner grants for both exact identities and
the matching active service delegation. It derives the owner solely from its
authoritative grant store. After verified predecessor removal, only the
retained original exact ownership binding may stand in for the removed
predecessor, and only for receipt access. A replacement incarnation or a
new owner cannot claim the receipt. Perform these checks before disclosing
even the presence of an operation and recheck before issuing a downstream
request on the successor's behalf or returning a retained receipt. Unavailable authorization state
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
| `scope` | `stop` for the destination POST, `receipt` for the destination GET on behalf of an authorized successor, or `reconcile` for the internal destination GET. |
| `delegation_id` | Exact canonical non-nil UUID of the grant. For internal reconciliation, use the original durably bound value. |
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
binding, excluding per-request scope and body digest. The destination trusts the
authenticated relay for a fresh grant check on every caller request.
A different grant cannot read or retry an operation bound to a revoked
grant. Internal reconciliation uses the mutually exclusive `reconcile`
scope and the exact retained operation, as defined below.

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
stop and may not clear the block. Once no delayed stop can act, a definitive
pre-effect refusal can release its own provisional lifecycle claim.
V1 retains the terminal exact-incarnation block indefinitely. A later
contract must define the no-old-write evidence and release procedure.

## Receipt and replay

At grant issuance the relay records the operation's absolute retry
deadline as its UTC issuance time plus 24 hours. One day covers retries
across a restart or brief outage. The relay and destination copy that
same deadline into their operation entries before allowing an effect.
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
same operation without another exit. A changed binding under the same
ID conflicts. A different ID cannot bypass an unresolved operation or
a retained admission block.

A relay refusal before its durable operation claim, including
`outbox-pending-replies` or `outbox-lifecycle-pending`, leaves no operation
entry. An authorized GET returns `stop-operation-not-found` for that ID.
The successor can retry the same ID after the cause clears while the
grant and deadline permit it. A definitive pre-effect refusal after the
durable claim is recorded as `refused` and replayed under that ID.
The destination must durably bar a delayed copy before its refusal can
settle a dispatched operation. A later attempt after the cause clears
requires a new operation ID bound by a new grant.

Use the same ID on transport loss and read back that operation. A missing
destination receipt after possible dispatch, malformed reply or timeout
is unresolved. Read the native outcome from the pinned destination.
Only the native durable operation entry and evidence against the
original exact runtime can settle a dispatched stop.

On an authorized caller POST retry before the deadline, the relay may
retransmit the original request under the same fence only after a
`receipt` read establishes that no effect was admitted and native
deduplication is available. A late copy contends for the same native
entry before any effect. When the native
entry shows that exit might have been submitted, reconcile from proof
without submitting it again. Every reconciliation GET is read-only and
never invokes stop or delete.

Stop and receipt replies have exactly `schema_version` (the stop schema
literal), `ok`, and either `data` or `error`. HTTP 200 carries `ok: true` and the
operation projection in `data`. A POST that verifies stop after exit
submission returns the terminal projection. An authorized GET returns
HTTP 200 for any retained projection. The projection has these exact
members.

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
and the permitted state and result before returning it. A POST that
verifies the predecessor was already stopped returns HTTP 409
`session-already-stopped` with its terminal receipt in
`error.details.receipt`. An exact POST replay keeps that status and
envelope. GET returns the same receipt in `data` with HTTP 200 and
`stop_result: "already_stopped"`. Before delivering either form, the
relay must durably record the admission block. The verified receipt
authorizes the successor to continue its existing retirement flow, which rechecks native stopped and unread state at delete.

### Internal settlement after revocation

The coordinating service revokes the grant after the successor's verified
receipt read-back. It may revoke early to withdraw authority. Revocation
immediately ends successor submissions and receipt access, including
projection of a result from an in-flight request. An exit already admitted
before revocation may settle. Preserve its evidence and fences.

On startup and when retrying a previously admitted pending or unknown
operation, the relay automatically reads the pinned destination using
the original durable operation binding and `scope: "reconcile"`. This
continues after revocation without a service trigger. The scope is
relay-internal and accepted only on the destination receipt GET with both
relay credentials. It has no caller-facing route and cannot admit a POST
or cause the relay to retransmit one.

For `reconcile`, the destination verifies the complete original binding
against its durable operation record, including the original
`delegation_id` and retry deadline, and returns only the native recorded
projection. This check uses the already-admitted operation instead of a
live grant. Missing or mismatched evidence cannot establish an outcome.
The destination's own recovery may settle that entry from exact native
proof without sending exit again. A result learned after revocation is
stored in the relay, with the normal fence transition, and never
projected to a revoked successor.

Retain terminal receipts outside the predecessor record in both stores
until both the retry deadline has passed and the coordinating service has
verified deletion of the exact predecessor by reading back authoritative
absence after conditional deletion. A different incarnation or missing
entry in an open list cannot establish that read-back. V1 stores retain
terminal evidence indefinitely. A later compaction contract must define
how both stores verify deletion read-back and issuer UTC expiry, and
must preserve a durable replay barrier for any ID that could otherwise
be admitted. A fast local clock cannot shorten retention. Receipt
compaction never releases the admission block. Pending and unknown
operations remain unresolved across the deadline.

## Refusals

After authenticating the hop, validate the POST media type and request
shape, then apply the route's authority check before revealing operation
or target state. Authentication and authorization refusals disclose no
operation or target state. Errors use `ok: false` and `error.code` only
except for the verified already-stopped receipt defined above. Return the
schema literal for the route.

| HTTP | Code | Condition |
| --- | --- | --- |
| 400 | `invalid-request` | Invalid selectors, version, headers other than the credentials or binding, or JSON shape. |
| 401 | `unauthorized` | Missing, invalid or repeated credentials for the hop, or any caller other than the configured coordinating service on grant issue or revoke. |
| 403 | `succession-stop-delegation-missing` | No service-issued grant authorizes this succession. |
| 403 | `succession-stop-delegation-revoked` | The matching grant was revoked. |
| 403 | `succession-stop-delegation-mismatch` | Unavailable caller incarnation, forbidden route, conflicting active grants, an operation outside the matching grant, or absent, invalid or mismatched caller attestation or destination binding. |
| 403 | `succession-stop-owner-mismatch` | The two exact session identities do not share an authorized owner. |
| 404 | `stop-operation-not-found` | No receipt exists for this authorized lookup; absence cannot settle an earlier dispatch. |
| 409 | `session-incarnation-conflict` | The exact predecessor record is absent or its current incarnation differs during stop admission. |
| 409 | `session-mailbox-not-empty` | The native unread predicate is false. |
| 409 | `session-already-stopped` | The native proof verifies that the exact predecessor was already stopped. |
| 409 | `session-stop-busy` | Native state or destination lifecycle contention prevents guarded stop. |
| 409 | `outbox-pending-replies` | Pending Console input could still reach the predecessor. |
| 409 | `outbox-lifecycle-pending` | The relay lifecycle slot or admission block excludes this operation. This is the relay contention code. |
| 409 | `operation-id-conflict` | An operation ID already binds different authorized values, or grant issuance reuses an ID with a different binding. |
| 409 | `stop-operation-expired` | An operation reached its recorded deadline before relay or native admission. |
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
- Verify grant issuance and revocation are service-only, old IDs cannot
  reactivate, and ambiguous grants fail closed. Revoke while a stop is
  unknown, then restart the relay. Verify it settles from the native
  receipt without sending exit or disclosing the receipt to the successor.
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
