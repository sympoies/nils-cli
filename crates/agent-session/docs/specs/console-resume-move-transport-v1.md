# Console resume-move v1 transport supplement

This document supplements [Console resume-move start v1](console-resume-move-v1.md),
frozen by [#4060](https://github.com/sympoies/nils-cli/pull/4060). It adds
discovery, a destination status read and the authenticated relay binding.
The v1 start routes, request and reply members, refusal allowlist, launch
key and no-replay rules are unchanged, except that the
[v1.1 erratum](console-resume-move-v1.md#version-11-erratum) removes
`caller.owner` from the start request. No route is enabled by this document.

## Capability discovery

Discover the literal capability `resume-move-start/v1` at each hop before
sending its start POST. Use the configured peer, authenticated transport
and exact route. Do not follow redirects, cache a positive answer across
operations or infer support from a version string, ordinary resume or an
open HTTP port.

| Client | Discovery | Positive evidence |
| --- | --- | --- |
| Managed origin client | `GET /sessions/{caller}/console-resume-move/v1` on its configured origin daemon, with the same local caller credential as start. | The origin authenticates the exact current caller and confirms local move-start support. |
| Origin daemon | `GET /api/coordination/resume-move/v1` on its configured Console relay, with the same origin relay bearer as start. | The relay authenticates the origin machine and confirms relay support. It still checks the specific caller grant and destination on POST. |
| Console relay | Existing `GET /sessions` on the pinned destination, using the configured machine transport. | An HTTP 200 serve envelope with `ok: true`, `data.machine` matching the pinned machine, and boolean `data.capabilities.resume_move_start_v1: true`. |

The two GET handshakes take no query or body. Their HTTP 200 reply is
exactly `schema_version: "agent-session.console-resume-move-capability.v1"`,
`ok: true` and `data`. The only data members are `machine` (the authenticated
origin machine in both replies) and `capability: "resume-move-start/v1"`.
A receiver verifies that identity against its configured origin. The existing
[session listing](serve-observation-v1.md) adds just the boolean capability.
It advertises destination support only when the v1 start and status routes,
native move support and configured relay ingress are enabled.

A missing route or capability, non-200 or malformed handshake, or wrong
machine is unavailable. For a first submission, return
`resume-move-unavailable` before forwarding to create. Never fall back
to an ordinary start, resume or existing resume-preflight route. After
possible dispatch, an unavailable read cannot change an unknown outcome
or reopen the move. Every POST receiver rechecks its own capability and
authorization. A discovery answer is not permission to start.

## Delegated caller binding

The existing [local caller capability or scoped worker delegation](scoped-worker-delegation-v1.md)
is authenticated only by the origin daemon. Do not send the caller credential
or delegation file to Console or the destination. The relay authenticates the
origin as its configured machine, takes the exact caller ID and incarnation
from the origin-attested start, and derives the owner and permitted
destination from its own caller grant. It rechecks that grant before
issuing each destination request, including a read for reconciliation.

Both destination routes require `Authorization: Bearer` with the existing
destination machine bearer, plus the configured `X-Agent-Session-Relay-Token`.
Use the distinct ingress credential and trust boundary already used by
[federated mailbox ingress](coordination-federation-v1.md#cross-host-mailbox-federation-v1).
The machine bearer alone never attests to a caller. Only the credentialed
relay may supply the binding below. The transport must protect credentials
in transit and must not follow redirects or expose headers to browsers.

The relay constructs one `X-Agent-Session-Resume-Move-Binding` header from
its authenticated route context. The value is unpadded base64url of a UTF-8
JSON object, at most 2 KiB encoded. This is an encoding, not a signature.
The exact JSON members are:

| Member | Value |
| --- | --- |
| `schema_version` | `agent-session.console-resume-move-binding.v1` |
| `origin_machine` | Machine authenticated by the relay's origin bearer |
| `caller_session_id` | Managed ID in the exact caller grant |
| `caller_session_incarnation` | Canonical UUID in the exact caller grant |
| `owner` | Owner derived from that grant. The request body has no owner member |
| `destination_machine` | Machine selected and allowed by that grant |
| `move_id` | Exact move UUID, canonical and non-nil |
| `scope` | Literal `read` for status GET or `start` for start POST |
| `operation_id` | Exact canonical Console UUIDv7 from the start body. Required for `start` and absent for `read`. |

All present members are non-null strings. Machine, caller ID and owner
use the exact validated selectors from configured routing and grants.
Reject duplicate credential or binding headers, invalid encoding and
unknown or duplicate members. The origin and relay do not accept
caller-supplied relay ingress or binding headers.

The destination verifies both credentials before parsing move selectors.
It validates the request shape, then checks the header schema, scope
against the method and route, exact local destination and move selector.
For start it also checks caller ID, caller incarnation, machine,
operation ID and move ID against the unchanged body. The owner comes only
from the binding, since the body has no owner member. For status it checks
the canonical move ID against the path.

On either route, invalid or missing credentials return HTTP 401 with
`resume-move-unavailable`. A missing, malformed or mismatched binding
returns HTTP 403 with `resume-move-binding-mismatch`. Refuse before any
move read or admission. Never echo the binding header or credentials.

The destination trusts the authenticated relay's attestation of the grant,
not the body or an arbitrary header. The caller is not the stopped source.
This binding cannot substitute for the signed native ticket or proof,
and grants no access outside the exact route and move.

## Destination status read

`GET /sessions/resume-move/{move_id}/status/v1` accepts a canonical, non-nil
move UUID in the path, no query and no body. It requires the two destination
credentials and a `read` binding for the exact move. It returns
`Cache-Control: no-store`. It reads the native move ledger and verifies
retained proof freshness locally. It never prepares, stages, writes a receipt,
acquires a writer, opens credentials for launch, or calls a create handler.

HTTP 200 has exactly `schema_version: "agent-session.console-resume-move-status.v1"`,
`ok: true` and `data`. All the following data members are required:

| Member | Value |
| --- | --- |
| `machine` | Exact configured destination machine |
| `move_id` | The exact request UUID |
| `state` | `prepared`, `staged`, `writer_admitted`, `created` or `aborted_no_writer`, from durable destination state |
| `session_id` | Exact managed ID `move-<move_id>` when `created`, otherwise null |
| `session_incarnation` | Exact managed runtime UUID recorded when `created`, otherwise null |
| `proof_freshness` | `fresh`, `expired`, `missing` or `unverified` |
| `proof_expires_at_epoch` | The verified stop-proof expiry as integer UTC Unix seconds for `fresh` or `expired`, otherwise null |

`fresh` means the retained stop proof and ticket verify, match the pinned
binding and satisfy native timing at the read. `expired` means those records
and their validity windows verify but the destination's UTC time is past
the proof's expiry (strictly greater than its epoch second). Use the
same boundary and skew rules as
[the native proof](resume-move-proof-v1.md#freshness-and-refusals).
`missing` means the move exists but has no retained stop proof.
`unverified` means the daemon cannot verify the evidence or its timing.
A deadline is never copied from an unverified claim. Do not infer expiry
from a ticket alone or mask a verified terminal outcome when proof
freshness is no longer positive.

Native `resume-move status` does not expose proof freshness or expiry.
The daemon implementation must add this projection from the verified
durable evidence. It must not compute it at the relay, change the native
CLI output, send the signed records or turn the read into a full admission
check. The destination still verifies signed binding, checkout, bytes and
writer state under the native admission lock on the separate start POST.

A read refusal has the same status schema, `ok: false` and `error`, with
the frozen `code` and `details` members and permitted detail values.
Return `safe_to_retry: false` and the frozen `next_action`. A read never
asserts `session_created: false`. Do not disclose the move ID or state
before authorizing that move.

| HTTP | Literal code | Condition |
| --- | --- | --- |
| 401 | `resume-move-unavailable` | Missing or invalid destination credentials. |
| 403 | `resume-move-binding-mismatch` | Missing, invalid or mismatched delegated binding after credential authentication. |
| 400 | `resume-unsupported` | Invalid route selector, query or body. |
| 404 | `resume-move-proof-missing` | No durable entry for the authorized move. |
| 422 | `resume-move-target-mismatch` | The native reservation is pinned to a different signer identity. |
| 422 | `resume-move-session-mismatch` | A recorded `created` identity does not match the required managed ID and incarnation shape. |
| 503 | `resume-move-state-unavailable` | The destination cannot read or validate the native outcome. |
| 503 | `resume-move-unavailable` | The move capability is disabled. |

### Using the read

Before the first destination start POST, require an authorized, unused
move key under the [durable dispatch fence](console-resume-move-v1.md#replay-and-read-only-reconciliation)
and a valid read of that pinned move. This internal status read does not
enable routed `--check-resume`. A verified typed read refusal is projected
with the frozen allowlist. For a successful read before first dispatch:

| Observed state and proof | Start decision |
| --- | --- |
| `staged` and `fresh` | May forward once under the durable dispatch fence. |
| `staged` and `expired` | Refuse with `resume-move-proof-expired`. |
| `prepared`, or `staged` and `missing` | Refuse with `resume-move-proof-missing`. |
| `staged` and `unverified` | Refuse with `resume-move-unavailable`. |
| `writer_admitted` | Return `resume-move-outcome-unknown`. |
| `created` | Refuse with `resume-move-destination-writer-present`. An initial read never claims Console ownership. |
| `aborted_no_writer` | Return `resume-move-aborted` and the frozen recovery hint. |

A malformed or unavailable read cannot permit the first start. The relay
can claim Console ownership after reconciliation only for a previously
dispatched, authenticated move bound to that owner.

After possible dispatch, read only the original destination under the
original authorized binding and receipt or compacted fence. Validate the
response's machine, move, state and created identity. Only valid durable
`created` or `aborted_no_writer` resolves the previously unknown outcome.
Return the original created identity or the existing native abort guidance.
A missing entry, expired proof, denied read, unconfirmed state or malformed
response never proves that no create occurred. Keep the binding and fence,
return `resume-move-outcome-unknown` with `safe_to_retry: false` where the
outcome remains unconfirmed, and never re-forward the POST.

## Implementation acceptance

- Drop, forge or misdirect each discovery answer. No unsupported or wrong
  hop receives a start POST and no fallback route is invoked.
- Reject missing or wrong ingress credentials, duplicate bindings, forged
  owners, and a read binding used on start. Confirm the start JSON is
  unchanged and no local caller credential leaves the origin.
- Verify the read is observational and projects signed proof expiry at
  the native boundary. Only a fresh staged observation can precede a
  first POST. Expired proof does not hide a durable created result.
- Lose the destination reply. Verify a later read returns the exact
  recorded identity or stays unknown, with no second dispatch.

Salon and nils-cli both sign off on the supplement. Implement it separately
and test-first before enabling routed moves.
