# Console attested session creation v1

This contract defines relay-authorized role attestation on
[`POST /sessions`](serve-session-create-v1.md) for
[#4077](https://github.com/sympoies/nils-cli/issues/4077). It covers fresh
Console roots and gives [role-based review-handoff supervisors](https://github.com/sympoies/nils-cli/issues/3853)
a way to obtain the same broker-verified role as a trusted local launch.
Runtime support follows in a separate test-first change.

## Console admission

Console authenticates the dispatch caller and resolves its exact current
caller grant and permitted destination using the trust boundary in the
[resume-move transport](console-resume-move-transport-v1.md#delegated-caller-binding).
An explicit server-side allowlist of authenticated dispatch callers maps
an eligible launch to the literal role `steward`. The relay checks that
policy for every attested dispatch. A caller may request an attested
launch, but supplies no identity or role value to attest. A managed
dispatcher may sponsor a root without becoming its lineage parent.

Console continues to apply its existing ownership, destination and
account admission. The relay constructs its destination request from
those decisions. Console retains its existing ownership of the session
and claims only the identity returned by the pinned destination. A
client cannot supply relay credentials, a binding, a forge initiator or
forge launch context. The relay rejects attempts to supply them before
dispatch and never forwards a local caller capability to the destination.

## Destination request

The relay sends `POST /sessions` to the configured destination over the
same protected machine transport as resume-move. It supplies exactly one
`Authorization: Bearer` with the destination machine bearer and exactly
one `X-Agent-Session-Relay-Token` with the distinct configured Console
relay ingress credential. The credentials are separately provisioned
and must differ. The destination verifies both before using the binding
or processing the body. The relay uses the pinned route, follows no
redirects, and keeps both credentials and the binding out of browsers,
logs and error responses.

Use `Content-Type: application/vnd.agent-session.console-attested-create.v1`
for the UTF-8 JSON body. This media type is outside the ordinary JSON
extractor's recognized types, so a destination without this contract
refuses the request before ordinary creation. A request with this media
type or the binding header selects attested admission. The relay sends
neither on ordinary creates. A refusal, unsupported media type, or
unconfirmed reply never authorizes an ordinary create or automatic
replay.

The relay sets exactly one `X-Agent-Session-Console-Create-Binding`
header from its authenticated request context. As in resume-move, the
value is unpadded base64url of a UTF-8 JSON object, with a maximum of
2 KiB encoded. This is an encoding. Authentication comes from the
protected transport and the two credentials. Use SHA-256 for request matching, as in the existing resume-move
transport contract.

| Member | Value |
| --- | --- |
| `schema_version` | Literal `agent-session.console-create-binding.v1` |
| `scope` | Literal `create`, valid only for the attested `POST /sessions` |
| `origin_machine` | Exact origin machine from the authenticated caller grant |
| `caller_session_id` | Exact managed ID from that grant |
| `caller_session_incarnation` | Exact canonical lowercase, hyphenated, non-nil UUID from that grant |
| `destination_machine` | Exact machine selected and authorized by the relay |
| `attested_role` | Literal `steward`, selected by server policy |
| `body_sha256` | 64 lowercase hexadecimal characters for the SHA-256 of the exact JSON body bytes sent to the destination |

All members are required non-null strings. Machine and caller ID use
the validated routing and grant selectors. Reject unknown and duplicate
members, repeated binding headers and invalid encoding. The daemon
checks the version, scope, exact local destination and digest of the
received bytes before admitting the create. The caller fields describe
the exact grant verified by the authenticated relay. They cannot
establish a parent or a session owner on the daemon.

The relay uses the ordinary fresh-create fields for launch inputs and
omits body `role`. The daemon accepts only fresh root creates for
attestation and builds the Console root lineage itself. If `lineage` is
present, it must pass the existing lineage validation and describe a
root. The daemon validates ordinary
launch inputs through existing admission checks, including account,
workspace and capacity. The attested request cannot import a provider
conversation, resume a session, or attach the role to an existing
record.

## Persistence and Forge consumption

At new-record persistence, the daemon sets the record's role to
`steward` and stores an immutable `LaunchContext` in
`lineage.forge_context` with `initiator: "console-relay"`,
`role: "steward"` and `attested_role: "steward"`. The initiator is a
daemon-owned literal. Neither the body nor the relay grant can choose
an initiator or claim an existing operator initiator. Store the launch
context with the new record before making the session available. The
record contains no relay credential, raw binding or dispatch-caller
identity from the header.

The existing
[broker identity projection](session-lineage-work-v1.md#forge-launch-context)
returns `attested_role` for the authenticated current session and
runtime incarnation. `forge-cli` reads that binding and verifies it
matches the acting session. For the role-based review-handoff selector,
`role:steward` matches the attested role independently of the initiator.
Repository policy and the other
[coordinator takeover checks](../../../forge-cli/docs/specs/forge-review-state-v1.md#coordinator-takeover)
still apply. Ordinary role labels and display data confer no supervisor
authority. A child has no inherited attestation. Existing lifecycle
operations preserve the original context on the same record, and each
replacement session requires its own authorized create.

## Refusals

These refusals precede record creation and launch. The destination uses
the ordinary serve error envelope. Authenticate first, then verify the
binding and role, then check the parsed body and regular create inputs.
Ordinary requests retain their existing authentication and validation.
Other routes cannot consume this binding as launch authority.

| Boundary | HTTP | Code | Condition |
| --- | --- | --- | --- |
| Console relay | 403 | `console-create-attestation-forbidden` | An authenticated caller requests attestation without a matching current role grant or uses a destination outside that grant. Advise using an authorized dispatch caller and destination. |
| Destination | 503 | `token-not-configured` | The destination machine bearer is unconfigured. |
| Destination | 401 | `unauthorized` | The machine bearer is missing or invalid, or an attested request has a repeated bearer or a missing, invalid or repeated ingress credential. |
| Destination or relay | 503 | `console-create-attestation-unavailable` | The authenticated attestation path is disabled, its distinct ingress credential is unavailable or equals the machine bearer, or the relay receives an unsupported-media-type refusal. |
| Destination | 403 | `console-create-binding-mismatch` | After credential verification, the required media type or binding is missing, malformed or repeated, or the binding version, scope, destination or body digest does not match. |
| Destination | 403 | `console-create-role-forbidden` | A validly shaped and matched binding proposes any role string other than the exact literal `steward`. |
| Destination | 400 | `console-create-invalid-body` | After binding and role verification the attested body is invalid UTF-8 or JSON, is not an object, or contains duplicate JSON keys. |
| Console relay or destination | 400 | `identity_session_binding_untrusted` | Any `lineage.forge_context` member, even if null. An attested request also refuses body `role` or any explicit forge identity field (`forge_context`, `initiator`, `attested_role`) at the root or in `lineage`. Console uses the same refusal for a client-supplied binding or ingress header. |
| Destination | 400 | `role-requires-root` | The attested create supplies valid parented session lineage. |
| Destination | 400 | `resume-unsupported` | The attested create requests a provider import or a resume. |

Ordinary bearer-authenticated creates may still carry an ordinary
`role` under the existing root rule. They cannot set a forge launch
context and still refuse `lineage.forge_context` with
`identity_session_binding_untrusted`. Regular create failures,
including an already used session ID, retain their existing codes.
The relay preserves these typed refusals and their status without
copying raw transport diagnostics.

## Threat model and implementation acceptance

- A holder of only the machine bearer keeps its ordinary machine
  permissions, including ordinary creation. It cannot mint attested
  launch context or provide a trusted relay binding. A holder of only
  the ingress credential cannot create a session or attest a role.
  A copy of the binding header alone grants neither permission.
- The relay is trusted to authenticate the exact caller, enforce its
  grant, and bind the role to the right body and destination. A stale
  caller grant, caller-provided headers, or a binding copied to a
  different body, machine or route cannot authorize an attested launch.
  Compromise of the relay or both credentials is outside this boundary.
  The role is provenance, and the existing same-OS-user launcher and
  private registry trust boundary still applies. Existing access to
  control a running session remains outside the create-time guarantee.
- Verify that a grant-matched request creates a fresh Console root with
  the server-set context and that its broker-authenticated
  `role:steward` satisfies the configured Forge selector. Exercise the
  same call from an ordinary `steward`-labeled session, from a child and
  with a stale broker incarnation. Each lacks a matching role binding.
- Verify the exact refusals for missing credentials, invalid bindings,
  a non-allowlisted role, caller-supplied identity, and child or resume
  creation. Confirm that none produces a session record or start. Send
  the attested media type to a destination that lacks support and
  verify it cannot create an ordinary session.

The contract requires security review and the Salon owner's exact-head
sign-off. The merged contract is the implementation reference. Enable
Console dispatch after the destination can admit the attested request.
