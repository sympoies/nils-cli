# Native client replies while a provider is working

This integration note complements the [Serve API v1](serve-api-v1.md) prompt
contract. A client that wants immediate provider handoff uses the authenticated
keyed prompt route with `require_idle` omitted or false:

```json
{
  "text": "Inspect the next failing test.",
  "expected_session_incarnation": "launch-id",
  "request_id": "reply-1",
  "require_idle": false
}
```

Send this body to `POST /sessions/{id}/prompt/v2`. A working activity state alone
does not require the client to retain the reply until a later idle observation.
The provider controls when accepted input affects its work. Native handoff does
not request Escape, an interrupt, compaction, or a replacement session.

## Provider paths

| Provider runtime | Immediate handoff | Acceptance limits |
| --- | --- | --- |
| Managed Codex app-server | `turn/start` on the exact bound thread through the existing control connection | The provider decides whether its active turn accepts additional input. The daemon requires a native acknowledgment; protocol errors or lost acknowledgments can remain unknown. |
| Supported Claude terminal | Existing structured pane submission to the exact runtime | The existing acknowledgment path observes a new provider turn or the native queued-input display. A running turn need not complete first. An unconfirmed submission remains unknown. |

The runtime support floors and lifecycle evidence are recorded in
[Provider turn signal evidence](../provider-turn-signal-evidence.md). They do
not guarantee acceptance in every provider state. For example, Codex documents
that an active review or manual compaction can reject additional same-turn
input in its [0.144.1 app-server protocol](https://github.com/openai/codex/blob/rust-v0.144.1/codex-rs/app-server/README.md).
Fixture protocol responses and modeled terminal queues demonstrate daemon
routing; they do not establish live acceptance by an installed provider.

## Fences and receipt recovery

Immediate replies retain incarnation, account, authorization, input validation,
and empty-composer checks. A genuine draft is preserved. Unsupported or
unobservable input state fails closed. Native receipt proof and reconciliation
are tracked separately in [issue #2710](https://github.com/sympoies/nils-cli/issues/2710).

`require_idle: true` remains the explicit guarded-send option. It requires an
idle provider and can return a definite busy refusal before delivery. A client
that selects immediate native handoff must not set that option or independently
wait for idle before issuing the request.

Keep the original request ID and expected incarnation when looking up or
repeating an uncertain attempt. A repeated keyed POST returns the retained
result without another dispatch. A changed payload under the same key is a
conflict. An unknown result, HTTP timeout, or lost connection does not authorize
a second dispatch under a fresh key. Receipt lookup is read-only.

A distinct follow-up can use a distinct request ID while an earlier key remains
unknown, provided the native connection and all admission fences remain valid.
Its acknowledgment belongs only to that later request. It neither resolves nor
replays the earlier one. This is independent request admission, not a guarantee
of model processing order; the provider owns that behavior.

Application cancellation can prevent an attempt only before handoff. After a
native attempt, cancellation must not imply retraction from the provider, and
an unknown original must remain unreplayed. This path adds no daemon-held
idle queue or cancel-after-handoff guarantee.
