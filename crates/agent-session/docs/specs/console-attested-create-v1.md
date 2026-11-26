# Console attested session creation v1

This contract defines relay-authorized role attestation on
[`POST /sessions`](serve-session-create-v1.md) for
[#4077](https://github.com/sympoies/nils-cli/issues/4077). It covers fresh
Console roots and gives [role-based review-handoff supervisors](https://github.com/sympoies/nils-cli/issues/3853)
a way to obtain a broker-verified steward role. Runtime support follows in
a separate test-first change.

## Console admission

The origin daemon authenticates the managed caller's current session
capability and runtime incarnation before sending a dispatch. The Console
relay authenticates that daemon using its configured origin bearer and
resolves the asserted caller ID and incarnation to the current caller on
that machine. Existing Console ownership, permitted-destination and account
checks also apply.

A grant is a reusable operator-managed entry in the Console relay
configuration. It maps the exact authenticated origin machine, caller
session ID and current session incarnation to the literal allowed role
`steward`. The relay checks that exact tuple and current caller authority on
every request, including an operation receipt lookup. A stale or unknown
incarnation fails admission. Removing the allowlist entry revokes it on the
next request. A request field, ordinary role label or caller's own
attestation supplies no grant and confers no authority to mint a steward.

The explicit client surface is the authenticated local
`POST /sessions/{id}/console-start/v1` API. An attested submission sets
the top-level JSON members `attest_role: true` and `no_parent: true`, and
supplies a valid explicit managed ID in `session.id`. The existing
`operation_id` and reconcile contract applies. An older local daemon rejects
the unknown attestation member before dispatch. Clients must treat that
refusal as unavailable and preserve the request's intent.

The origin sends `attest_role: true` on
`POST /api/coordination/sessions/v1` at the top level beside
`source_session_id`, `source_incarnation` and `no_parent: true`. The literal
case-sensitive key `attest_role` is valid only at the top level and only
once, with a JSON boolean value. An omitted field or false selects ordinary
creation. A different spelling, a nested field, duplicate members or a
non-boolean value is invalid. An ordinary role label does not opt in.
Attested requests require a fresh root and refuse any supplied role key,
including null or an empty string. A managed dispatcher can sponsor the
root without becoming its lineage parent. The relay consumes the intent
and root flag and omits them from the destination request.

### Capability discovery

Before dispatching an attested create, the origin MUST discover support
from its configured relay with authenticated
`GET /api/coordination/attested-create/v1`, with no query or body.
The relay uses the existing origin bearer and returns HTTP 200 with
`schema_version: "agent-session.console-attested-create-capability.v1"`,
`ok: true` and `data` containing the authenticated origin `machine` and
`capability: "console-attested-create/v1"`. The origin verifies the response
shape and the machine against its configured origin. Recheck before each
dispatch. A missing route, invalid or missing capability or non-200 reply
returns `console-create-attestation-unavailable` before a create POST.
The origin MUST NOT drop the intent or fall back to ordinary creation.

### Operation and replay

Attested create requires an explicit `session.id` and binds it to the
existing [Console operation receipt](coordination-runtime-v1.md#operation-diagnostics-and-reconciliation).
Use the canonical UUIDv7 operation identity and existing freshness rules.
The origin retains the operation identity for retries. Before possible
dispatch, bind the receipt to the authorized caller and incarnation, the
pinned destination, the requested ID, and the exact validated launch
inputs including attestation intent. The relay must commit the dispatch
claim before sending a destination create. Unavailable or uncertain
receipt writes prohibit dispatch.

Recheck the grant before using an existing receipt. Key the attested
dispatch fence by the pinned destination and requested managed ID. An
exact retry replays or reconciles its receipt without another destination
create. Reusing either bound identity with changed inputs conflicts.
Preserve the fence through relay restart and receipt expiry. A stale
operation, unavailable receipt or unconfirmed outcome cannot reopen the
fence, even with a fresh operation ID. Receipt reconciliation is read-only.

The relay retains attribution in the private operation receipt, including
the authenticated origin and caller incarnation, pinned destination, and
the confirmed session identity. It claims ownership and records the start
mark only after confirming the exact attested result. Neither a local
caller capability nor caller-supplied relay credentials, binding, role or
forge identity may cross into the destination request.

## Destination request

The relay sends `POST /sessions` to the configured destination. The
transport MUST protect credentials in transit, MUST NOT follow redirects
and MUST NOT expose credentials or bindings to browsers, logs, runtime
environments or error responses. These requirements apply to the
origin-to-relay and relay-to-destination hops.

On this route the relay supplies exactly one `Authorization: Bearer` with
the destination machine bearer and exactly one
`X-Agent-Session-Attested-Create-Token` with the dedicated attested-create
credential for that destination. The destination configures this secret
locally as `AGENT_SESSION_CONSOLE_ATTESTED_CREATE_TOKEN`. Only the Console
relay and the target daemon hold it. The Console relay selects it from
its per-destination credential map. Each destination MUST use its own
unique credential, distinct from the machine bearer and the federated
mailbox `AGENT_SESSION_RELAY_INGRESS_TOKEN`.
An absent, unconfigured, invalid or reused credential cannot authorize
attestation.

The attested-create credential is accepted only for the attested media
type and scope on `POST /sessions`. It grants no authority on ordinary
create, mailbox ingress or resume-move.

Rotate or revoke by changing the local credential configuration on the
destination and the relay's matching destination entry. The destination
loads its credential at daemon start. Applying a changed or removed
credential requires a daemon restart. A partially updated pair fails
closed.

The attested media type is
`application/vnd.agent-session.console-attested-create.v1`, with a UTF-8
JSON body. It MUST NOT use a `+json` suffix or a generic JSON media type.
An unsupported destination MUST reject the attested media type with HTTP
415 before ordinary create admission. A request carrying the attested
media type, token header or binding header selects attested admission.
The relay sends none of these markers for ordinary create. Even if an
intermediary rewrites the media type and strips all distinguishing
headers, the forwarded body contains no authority to attest. At most
ordinary creation can occur. Both the relay and origin reject the
ordinary reply as unconfirmed, and never retry through ordinary create.

The relay sets exactly one `X-Agent-Session-Console-Create-Binding` from
its verified dispatch context. The value is unpadded base64url of a UTF-8
JSON object, limited to 2 KiB encoded. Authentication comes from the
protected transport and the two credentials. Serialize the body once and
compute `body_sha256` over the exact bytes that the relay sends.

| Member | Value |
| --- | --- |
| `schema_version` | Literal `agent-session.console-create-binding.v1` |
| `scope` | Literal `create`, valid only for the attested `POST /sessions` |
| `origin_machine` | Exact origin machine authenticated by the relay |
| `caller_session_id` | Exact managed ID matched by the relay grant |
| `caller_session_incarnation` | Exact canonical lowercase, hyphenated, non-nil UUID matched by the relay grant |
| `destination_machine` | Exact machine selected and authorized by the relay |
| `attested_role` | Literal `steward`, selected from the matched grant |
| `body_sha256` | 64 lowercase hexadecimal characters for SHA-256 of the exact received body bytes |

All members are required non-null strings. Reject unknown or duplicate
members, repeated binding headers and invalid encoding. Verify the
version, scope, exact local destination and body digest before using the
binding. Caller fields attribute the request to the grant verified by
the relay. They convey neither lineage parentage nor session ownership
to the daemon.

The relay sends normal fresh-create launch inputs and the exact mandatory
`id` and omits caller-supplied lineage. A caller-supplied role or forge
identity is refused. Forge identity fields include `forge_context`, `initiator` and
`attested_role` at the root or within lineage. Both
relay and destination refuse non-empty caller lineage. An absent, null
or empty lineage object is treated as empty and never stored. The
destination constructs and stores its own Console root lineage. Account,
workspace, capacity and other launch inputs pass normal create checks.
An attested request cannot import a provider conversation, resume an
existing session or attach the role to an existing record.

## Persistence and confirmation

At new-record persistence, the daemon sets `role` to `steward` and writes
an immutable `lineage.forge_context` with `initiator: "console-relay"`,
`role: "steward"` and `attested_role: "steward"`. The initiator literal
`console-relay` is reserved for this daemon path and is unavailable to
operator-selected initiator IDs. Persist the launch context before
making the record available. The record carries no relay credential,
raw binding or dispatch-caller identity from the header.

The destination success is HTTP 201 and carries the resulting session
projection. Before claiming ownership or returning success, the relay
verifies the pinned destination's response, including the exact requested
`session.id`, `session.role: "steward"` and all three required members of
`session.lineage.forge_context`. The origin verifies the same fields in
the relay reply before reporting success. A missing, malformed, mismatched
or ordinary success response, a transport failure, or any reply that
fails these checks leaves the outcome unconfirmed unless a definitive
typed refusal was verified. Return `console-create-attestation-unavailable`
and reconcile the existing receipt. An unconfirmed reply claims no
ownership and permits neither another launch nor a fallback.

The existing
[broker identity projection](session-lineage-work-v1.md#forge-launch-context)
returns `attested_role` for the authenticated current session and runtime
incarnation. `forge-cli` verifies that the binding matches the acting
session. The review-handoff selector `role:steward` matches the attested
role independently of the initiator. Repository policy and the other
[coordinator takeover checks](../../../forge-cli/docs/specs/forge-review-state-v1.md#coordinator-takeover)
still apply. Each replacement session needs its own attested create.
Children receive no inherited attestation. Lifecycle operations preserve
the original context on the same record.

## Refusals

The destination uses the ordinary serve error envelope. For attested
admission, check the machine bearer before looking up or reporting the
dedicated credential configuration. Once bearer authentication succeeds,
check the dedicated credential, media type, binding and role in that
order, then check the strict JSON body and ordinary launch constraints.
At the relay, authenticate the origin before inspecting dispatch members;
validate their shape, then check the exact caller and grant before any
forwarding or receipt disclosure. Reuse the static message `invalid
attested create request` for the invalid-body code at either boundary.

| Boundary | HTTP | Code | Condition |
| --- | --- | --- | --- |
| Console relay | 400 | `console-create-invalid-body` | Invalid dispatch shape, including an unknown, nested, repeated or non-boolean `attest_role`, or an invalid or missing explicit ID on opt-in. |
| Console relay | 403 | `console-create-attestation-forbidden` | Missing or revoked exact grant, stale or unknown caller incarnation, or forbidden destination. |
| Destination | 503 | `token-not-configured` | The destination machine bearer is unconfigured. |
| Destination | 401 | `unauthorized` | The machine bearer is missing, invalid or repeated, or the configured attested-create credential is missing from the request, invalid or repeated. |
| Destination | 415 | Unsupported media type | The destination refuses the request media type before launch. The relay maps a destination 415 to `console-create-attestation-unavailable`. |
| Destination, relay or origin | 503 | `console-create-attestation-unavailable` | The attested path or required credential configuration is unavailable, required discovery fails, or the result is unconfirmed. |
| Destination | 403 | `console-create-binding-mismatch` | After both credentials and the media type are verified, the required binding is absent, invalid, repeated or mismatched. |
| Destination | 403 | `console-create-role-forbidden` | A validly shaped and matched binding proposes a role other than `steward`. |
| Destination | 400 | `console-create-invalid-body` | After binding and role verification, the body is invalid UTF-8 or JSON, is not an object, contains duplicate keys, or lacks a valid explicit ID. |
| Console relay or destination | 400 | `identity_session_binding_untrusted` | Caller-supplied forge identity or any present body `role` key on attested create, or caller-supplied attestation headers at the relay. |
| Console relay or destination | 400 | `role-requires-root` | Attested create lacks the required root intent at the relay or supplies non-empty lineage at either hop. |
| Console relay or destination | 400 | `resume-unsupported` | Attested create requests a provider import or resume. |

Ordinary bearer-authenticated creates may carry a descriptive role under
the existing root rule. They still refuse any `lineage.forge_context`
member, including null, with `identity_session_binding_untrusted`.
Regular create failures, including an already used ID, retain their
existing codes. Preserve verified typed refusals and their status across
the relay, except for the unsupported-media-type mapping above. Do not
copy raw diagnostics.

## Threat model and implementation acceptance

- The origin daemon is trusted to verify the local caller capability
  and live incarnation. The relay trusts the configured origin bearer
  to attest the origin and enforces the exact operator grant. Compromise
  of the origin bearer defeats the caller authentication on which grant
  matching relies. The relay is trusted with each destination's dedicated
  credential and machine bearer.
- A holder of only a machine bearer retains ordinary machine
  permissions. A holder of only a dedicated attested-create credential,
  mailbox ingress token or copied binding cannot attest a role. The
  destination trusts an authenticated relay for the exact caller and
  body. Compromise of the relay, both destination credentials, or the
  same-user trusted launcher/private registry is outside the boundary.
- Verify grant admission and revocation, a stale or unknown incarnation
  and a caller who has the ordinary or attested `steward` role but lacks
  the operator grant. Verify the exact root-only checks and invalid-body
  cases, including wrong-case, nested and duplicate intent and null role.
- Use the preceding released daemon in a synthetic loopback fixture.
  Send the exact attested media type to `POST /sessions` with a valid
  fixture machine bearer and otherwise ordinary valid create JSON.
  Require HTTP 415 and verify that the session-record and launch counts
  remain zero. Against an older origin and relay, require pre-dispatch
  refusal and zero create POSTs for explicit attestation intent.
- With valid and invalid machine bearers, verify that dedicated-credential
  availability is disclosed only after bearer authentication. Exercise
  missing and duplicate credentials and confirm zero launch.
- Strip a binding, rewrite Content-Type, then drop all distinguishing
  headers in a synthetic proxy. None of those cases may persist an
  attested launch or produce ownership or a confirmed success. Reject
  replies with the wrong ID, role or attested context. Verify that a
  dropped reply and exact retry can only reconcile the first attempt.
- Verify that a matching grant creates one fresh Console root with
  server-set context and that the authenticated broker projection
  satisfies the configured Forge role selector. Check that the dedicated
  credential is scoped to one destination and refuse disabled, rotated
  away, shared or wrong-purpose credentials.

Merge requires security review and the Salon owner's exact-head sign-off.
Enable attested dispatch after both relay capability discovery and
destination attested admission are available.
