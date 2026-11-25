# Console resume-move start v1

This contract defines how Console starts an already prepared cross-host move.
It does not enable the route or change direct local starts, as described in [Cross-host resume proof v1](resume-move-proof-v1.md).

## Routes and authority

The capability is `resume-move-start/v1`. All three start routes use
`agent-session.console-resume-move.v1` as the JSON schema version.

| Hop | Start route | Authority |
| --- | --- | --- |
| Managed caller to origin daemon | `POST /sessions/{caller}/console-resume-move/v1` | Authenticate the managed caller for this operation. The path ID and body binding must match its authenticated ID, current incarnation and owner. |
| Origin daemon to Console relay | `POST /api/coordination/resume-move/v1` | Authenticate the origin and verify its delegated scope for the caller, owner and explicit destination. |
| Console relay to destination daemon | `POST /sessions/resume-move/v1` | Authenticate the relay and verify the delegated caller, owner and route scope. Only the destination daemon may admit the move. |

Each hop validates capability, version, shape and scope before forwarding
or admitting the request. The authenticated route resolves the explicit
`machine` to the destination daemon and its expected move signer identity
through local configuration. Neither request nor reply can override this
mapping. Missing capability on any hop or an unverifiable route returns
`resume-move-unavailable` before possible dispatch to create. There is no
fallback to ordinary Console start or ordinary resume.

V1 covers managed origins and outcome reconciliation only. Routed
`--check-resume` stays refused, and reconciliation must never invoke a
create route. Source stop and retain
([#4058](https://github.com/sympoies/nils-cli/issues/4058)), export,
transfer, staging, abort, source recovery and settlement stay outside
the relay's authority.

## Start request

These are the only JSON members. All are required and non-null. Reject
unknown or duplicate fields at any level. UUIDs use lowercase hyphenated
text. Forward the validated values unchanged between hops. Authentication uses the hop's existing
authenticated transport rather than credentials in the body.

| Object | Literal key | Type and meaning |
| --- | --- | --- |
| Root | `schema_version` | The exact version string above. |
| Root | `operation_id` | Canonical Console UUIDv7. Correlates receipts, not launches. |
| Root | `machine` | Nonempty destination selector. Only verified configuration can bind it to a destination. |
| Root | `caller` | Object with the three caller members below. |
| Root | `session` | Object with the five session members below. |
| `caller` | `session_id` | Managed caller ID, matching `{caller}` on the origin route. |
| `caller` | `session_incarnation` | Canonical UUID of the authenticated caller incarnation. |
| `caller` | `owner` | Nonempty identifier bound by authentication. |
| `session` | `move_id` | Canonical UUID. The single launch key. |
| `session` | `agent` | Literal `codex` or `claude`. |
| `session` | `resume` | Canonical provider conversation UUID. |
| `session` | `account` | Explicit, nonempty broker account alias. |
| `session` | `cwd` | Nonempty destination checkout selector, not authority to open or launch from a path. |

The caller binding authorizes the operation and receipt access. It does
not identify the stopped source or prove that it stopped. The session
fields are assertions. The destination matches them against its pinned
reservation, signed ticket and stop proof, verified checkout identity
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
regardless of JSON member order or insignificant whitespace. The relay atomically claims
the receipt indexed by verified destination and `move_id`, scoped to the
admitted owner. Bind each authenticated operation ID to the same receipt.
Check authorization before returning an outcome or reporting a conflict.
A binding conflict does not establish the outcome of the original start.

An exact repeat, even with a new operation ID, returns the retained
session identity or terminal refusal, or reconciles read-only. It never
forwards a second start. Changed bound values conflict, including an
attempt to retarget the same move. A failed or uncertain receipt write
must not result in forwarding. Restarts preserve the dispatch fence.
An absent or expired receipt is not proof of non-dispatch and cannot
reopen a previously submitted move for forwarding.

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

- Verify that all allowed values reach the destination unchanged.
  Extra or duplicate fields, forged caller bindings and unverified routes
  fail before create. Missing capability sends no fallback POST.
- Race native admission against abort. Only the destination decides the
  outcome under its lock. A completed start persists the exact identity
  and proof-derived lineage.
- Verify that a typed refusal and its classification survive both hops.
  Unknown codes or malformed outcomes reconcile read-only without a
  no-create claim or raw error details.
- Repeat an exact request after a terminal refusal and verify the retained
  refusal is returned. Changing a bound field or destination must conflict
  without a second dispatch.
- Drop a reply after dispatch, restart a hop and repeat with a new
  operation ID. Verify at most one launch, the original created identity
  on read-only reconciliation, and no re-forward while outcome is unknown.
