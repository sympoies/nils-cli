# Console resume-move start v1

This contract defines how Console starts an already prepared cross-host move.
It does not enable the route or change direct local starts, as described in [Cross-host resume proof v1](resume-move-proof-v1.md).

## Version 1.1 erratum

Version 1.1 removes `caller.owner` from the start request. The relay's
caller grant is the only owner authority, so a body assertion adds nothing
the grant does not already decide. The schema version string stays
`agent-session.console-resume-move.v1`. Version 1.1 is the first revision
that carries no owner member, and the Salon relay parser changes in the same
release. The erratum also states that `resume_move` lineage appears only for
bearer-authenticated session listings. Nothing else on the wire changes.

## Routes and authority

The capability is `resume-move-start/v1`. All three start routes use
`agent-session.console-resume-move.v1` as the JSON schema version.

| Hop | Start route | Authority |
| --- | --- | --- |
| Managed caller to origin daemon | `POST /sessions/{caller}/console-resume-move/v1` | Authenticate the managed caller for this operation. Match the path and caller IDs and the caller incarnation to the authenticated runtime. |
| Origin daemon to Console relay | `POST /api/coordination/resume-move/v1` | Authenticate the origin and resolve the exact caller grant. Verify the destination against that grant and derive the owner from it. |
| Console relay to destination daemon | `POST /sessions/resume-move/v1` | Authenticate the relay and verify the delegated caller, derived owner and route scope in the authenticated relay binding. Only the destination daemon may admit the move. |

Each hop validates capability, version, shape and scope before forwarding
or admitting the request. The authenticated route resolves the explicit
`machine` to the destination daemon and its expected move signer identity
through local configuration. Neither request nor reply can override this
mapping. Missing capability on any hop or an unverifiable route returns
`resume-move-unavailable` before possible dispatch to create. There is no
fallback to ordinary Console start or ordinary resume.

The relay derives the owner from its own grant for the authenticated origin
machine, caller session ID and caller incarnation. That grant is the only
owner authority. The request carries no owner member, and a managed caller
never supplies or learns one. A request that still carries a `caller.owner`
member is refused with `resume-move-binding-mismatch` before dispatch, in
place of the generic unknown-member refusal below. The destination `machine`
must be in the grant principal's permitted machines. Refuse a forbidden
target with `resume-move-unavailable` before dispatch. The origin binding
authenticated by the destination carries the relay's derived owner as its own
member, never from the request body.

V1 covers managed origins and outcome reconciliation only. Routed
`--check-resume` stays refused, and reconciliation must never invoke a
create route. Source stop and retain
([#4058](https://github.com/sympoies/nils-cli/issues/4058)), export,
transfer, staging, abort, source recovery and settlement stay outside
the relay's authority. [Console succession stop](console-succession-stop-v1.md)
defines a separate route for regular start succession.

## Start request

These are the only JSON members. All are required and non-null. Reject
unknown or duplicate fields at any level. UUIDs use lowercase hyphenated
text. Forward the validated values unchanged between hops. Authentication
uses the hop's existing authenticated transport rather than credentials
in the body.

The managed origin client mints `operation_id` once before sending the first
POST to the origin daemon. Every later hop validates the UUIDv7 and
preserves it unchanged. No daemon or relay substitutes an ID or generates
one for a missing field.

| Object | Literal key | Type and meaning |
| --- | --- | --- |
| Root | `schema_version` | The exact version string above. |
| Root | `operation_id` | Canonical Console UUIDv7. Correlates receipts, not launches. |
| Root | `machine` | Nonempty destination selector. Only verified configuration can bind it to a destination. |
| Root | `caller` | Object with the two caller members below. |
| Root | `session` | Object with the five session members below. |
| `caller` | `session_id` | Managed caller ID, matching `{caller}` on the origin route. |
| `caller` | `session_incarnation` | Canonical UUID of the authenticated caller incarnation. |
| `session` | `move_id` | Canonical UUID. The single launch key. |
| `session` | `agent` | Literal `codex` or `claude`. |
| `session` | `resume` | Canonical provider conversation UUID. |
| `session` | `account` | Explicit, nonempty broker account alias. |
| `session` | `cwd` | Nonempty destination checkout selector, not authority to open or launch from a path. |

The authenticated caller and its grant authorize the operation and receipt
access and supply the owner. The body binding alone does neither. It does
not identify the stopped source or prove that it stopped. The session fields
are assertions. The destination matches them against its pinned reservation, signed ticket and stop proof, verified checkout identity
and exact staged bytes. It launches from its verified local cwd.

V1 accepts no prompt, managed ID, alternative account key, provider
override, provider arguments or profile, or launch environment. Title,
work and Console parent metadata are not wire fields and cannot affect
admission. Proof bytes, staging and provider-home paths, history,
credentials and raw diagnostics stay off the relay.

## Destination admission and lineage

The destination dispatches to the native move start engine under its
destination control lock, which serializes admission against abort. It
uses the native default managed ID `move-<move_id>`. It persists
`writer_admitted` before resolving credentials, publishing provider
state or launching. A relay acknowledgement or positive preflight does
not admit a writer.

At managed-record creation, the daemon stores the following
`resume_move` lineage, projected from the admitted proof and signed
binding rather than from caller metadata.

| Key | Value |
| --- | --- |
| `schema_version` | `agent-session.resume-move-lineage.v1` |
| `move_id` | Move UUID |
| `source_host` | Verified source signer identity |
| `source_session_id` | Retained source managed ID |
| `source_incarnation` | Verified source runtime incarnation UUID |
| `source_generation` | Verified source generation from the proof |
| `provider`, `provider_session_id` | Verified provider and conversation UUID |

Move lineage is separate from Console ownership, parent and work lineage
and contains no credential, path or raw proof. Only the destination may
record `created`, and only for the exact resulting ID and incarnation.

After verifying `created` against the pinned machine, move binding and
exact `move-<move_id>` managed ID, the relay records Console ownership and
the start mark for that machine, ID and returned incarnation in the Console
edge grant store. It uses the owner derived from the authenticated caller
grant. The destination daemon does not stamp Console ownership, and the
relay never uses an owner field on the daemon record. The claim is
idempotent and cannot replace a different owner. Reconciliation can
complete that Console bookkeeping from the same verified outcome without
mutating the destination.

The destination exposes `resume_move` on the exact session record in
`GET /sessions`, and only when that request is authenticated with the
machine bearer. The open, unauthenticated response never carries
`resume_move`, so an unauthenticated caller never sees the source host,
source session or provider conversation IDs. Console uses the verified
source signer identity, source managed ID and source incarnation to map the source
record to that destination session. It resolves the signer through the
configured machine binding, not a name match or caller metadata. That
mapping lets Console carry its own metadata forward under its ownership
rules. Move lineage itself grants no access.

## Replies and refusal projection

The success envelope has exactly `schema_version`, `ok: true` and
`data`. The `data` object has exactly `machine`, `move_id`,
`state: "created"`, `session_id` and `session_incarnation`. Return the
managed ID and runtime incarnation UUID from the durable `created`
outcome. Each receiver checks the machine, move binding and exact
default managed ID before accepting success.

A typed refusal envelope has exactly `schema_version`, `ok: false` and
`error`. The `error` object has exactly `code` and `details`. Do not
forward upstream message text or arbitrary details.

| Allowed `details` key | Presence and value |
| --- | --- |
| `move_id` | Required for a validated request. Must match the request. |
| `state` | Optional, only when verified at the destination. One of `prepared`, `staged`, `writer_admitted`, `created` or `aborted_no_writer`. |
| `safe_to_retry` | Required, always `false`. |
| `session_created` | Include only as `false` for a verified no-create refusal. Omit for an unconfirmed outcome. |
| `next_action` | Required. One of the literal actions below. |

Preserve the exact allowlisted code, HTTP refusal status and verified
no-create or unknown classification through both hops and the origin
response and receipt. HTTP status alone never proves no create.
Authenticate before returning move details. Reject invalid request
shape before dispatch with `resume-unsupported`. Use
`resume-move-binding-mismatch` for a conflict with a bound request.

| Literal codes | `next_action` for a verified refusal |
| --- | --- |
| `resume-unsupported`, `resume-account-mismatch`, `resume-history-missing`, `session-exists` | `inspect_destination` |
| `resume-source-live`, `resume-source-unknown`, `resume-move-source-fenced`, `resume-move-source-busy`, `resume-move-session-mismatch`, `resume-move-source-already-stopped` | `inspect_source` |
| `resume-move-proof-invalid`, `resume-move-proof-expired`, `resume-move-proof-missing`, `resume-move-binding-mismatch`, `resume-move-target-mismatch`, `resume-move-path-mismatch`, `resume-move-transcript-cwd-outside-checkout`, `resume-move-history-stale`, `resume-move-history-unsupported`, `resume-move-destination-reserved` | `inspect_destination` |
| `resume-move-ssh-keygen-unavailable`, `resume-move-trust-unavailable`, `resume-move-signing-unavailable`, `resume-move-state-unavailable`, `resume-move-unavailable` | `inspect_destination` |
| `resume-move-destination-writer-present`, `resume-move-outcome-unknown` | `reconcile_destination` |
| `resume-move-aborted` | `obtain_no_writer_receipt` |

This is a literal list, not prefix matching. It already includes the
approved #4058 source refusals without adding any relay source operation.
The abort hint means obtain the signed no-writer receipt on the
destination with `resume-move abort`, then use it for native source
recovery. Neither the hint nor a relay receipt authorizes recovery.

After possible dispatch, an unknown code, malformed or missing reply,
or unconfirmed outcome is unresolved. If no valid typed outcome was
received, report `resume-move-outcome-unknown`. Every unresolved result
uses `reconcile_destination` and `safe_to_retry: false`, with no
`session_created` claim. Never apply ordinary `start-failed` cleanup or
retry heuristics to a move.

## Replay and read-only reconciliation

Before forwarding, the origin and relay durably persist an in-flight
receipt and a SHA-256 digest of the canonical validated request excluding
`operation_id`. Each store computes a stable digest over the same values
regardless of JSON member order or insignificant whitespace. The digest
covers `machine`, `caller.session_id`, `caller.session_incarnation` and the
five `session` members, so a repeat from a restarted caller conflicts and is
reconciled with the destination's native `resume-move status`. The relay
atomically claims the receipt indexed by verified destination and `move_id`, scoped to the
admitted owner. Bind each authenticated operation ID to the same receipt.
Check authorization before returning an outcome or reporting a conflict.
A binding conflict does not establish the outcome of the original start.

An exact repeat, even with a new operation ID, returns the retained
session identity or terminal refusal, or reconciles read-only. It never
forwards a second start. The origin reconciles by re-sending its retained
original request to the relay, which answers from its own fence. V1 has no
separate relay read route. Changed bound values conflict, including an
attempt to retarget the same move. A failed or uncertain receipt write
must not result in forwarding. Restarts preserve the dispatch fence.
Authorize a first dispatch only after atomically claiming an unused
move key in an intact durable index. An absent hop receipt is not
evidence that the key is unused.

Retain relay and destination move receipts at least through the original
destination's verified stop-proof expiry. The relay uses authenticated
read-only evidence from the pinned destination to confirm freshness before
a first forward and to determine when receipts may be compacted. It does
not receive the proof bytes or use a create request to check freshness.

After expiry, the relay may compact the full receipt only while retaining
the move's dispatch fence, bound owner, destination, digest and terminal
result or unresolved marker. An expired operation ID cannot reopen a move.
With no receipt, a destination-confirmed expired proof is a
`resume-move-proof-expired` refusal, not a first submission. If prior
dispatch is known or cannot be ruled out, expiry does not establish no
create or end reconciliation. If the store or verified freshness evidence
is unavailable, do not dispatch.

Read-only reconciliation uses the original pinned destination and exact
`move_id` under the same owner and route checks. Use the native outcome
reported by destination `resume-move status --move-id UUID`, never a
Console list or managed-name match.

| Verified outcome | Result |
| --- | --- |
| `created` | Return the original ID and incarnation, even if the runtime is no longer live. |
| `aborted_no_writer` | Return `resume-move-aborted`. The source still needs the exact signed native receipt. |
| `writer_admitted` without a recorded result | Return `resume-move-outcome-unknown` and leave the source fenced. |
| `prepared`, `staged`, unavailable or missing after possible dispatch | Remain unknown. A delayed request may still arrive. |

A dropped, timed-out or malformed reply never authorizes forwarding the
submission again. Receipt retention must preserve the dispatch fence
even when the hop receipt cannot be read.

## Rollout and acceptance

The nils-cli and Salon owners both sign off before this spec merges.
The merged nils-cli commit is the frozen reference. Salon supports that
version first. A separate test-first nils-cli change then implements the
daemon and CLI boundary and lifts the `--via-console` refusal only for
end-to-end capability. This specification changes no runtime behavior.

- Verify that all allowed values, including the client-minted operation
  ID, reach the destination unchanged. Extra or duplicate fields, forged
  caller bindings and forbidden destinations
  all fail before create. A request carrying `caller.owner` is refused with
  `resume-move-binding-mismatch`. Missing capability sends no fallback POST.
- Race native admission against abort. Only the destination decides the
  outcome under its lock. A completed start persists the exact identity
  and proof-derived lineage. The relay claims that incarnation for the
  grant-derived owner in Console, and Console maps its source through the
  bearer-authenticated listing without granting rights from lineage.
  The open listing never carries `resume_move`.
- Verify that a typed refusal and its classification survive both hops.
  Unknown codes or malformed outcomes reconcile read-only without a
  no-create claim or raw error details.
- Repeat an exact request after a terminal refusal and verify the retained
  refusal is returned. Changing a bound field or destination must conflict
  without a second dispatch.
- Drop a reply after dispatch, restart a hop and repeat with a new
  operation ID. Verify at most one launch, the original created identity
  on read-only reconciliation, and no re-forward while outcome is unknown.
  Repeat after proof expiry and receipt compaction to verify the same
  fence. No receipt plus an expired proof must never forward a start.
