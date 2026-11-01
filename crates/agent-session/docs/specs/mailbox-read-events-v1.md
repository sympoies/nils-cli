# Mailbox read events v1

Part of [Serve history and archive v1](serve-history-v1.md). A session's mailbox
reads and acknowledgements appear in its conversation as display-only timeline
events. The event is evidence that the recipient read or acknowledged a message.
It is never prompt text, notice input, notice delivery or a new input transport,
so recording or showing it cannot start an agent turn.

The wire grammar and parser are [Session notices v1](session-notices-v1.md).
This contract reuses its display encoding and parser only. The
[fixture corpus](../../tests/fixtures/mailbox-read-events-v1.json) pins exact
JSON for consumers; a consumer needs no Rust to use it.

## Event identity and lifecycle

One logical mailbox message has at most one event per provider conversation.
The identity is the stored `message_id`; resume carry and forwarding move a
message in place and keep it. Recording is idempotent by that identity.

| Recipient action | Event effect |
| --- | --- |
| First successful `message show` | Creates the event: `state` `read`, `read_at` and `timestamp` set to the commit time, `revision` 1. |
| Any later `message show` | No change. No second event, body, position, `read_at` or `revision`. |
| First successful `message ack` | Sets `state` `acknowledged` and `acknowledged_at`; `revision` advances by one. Body, `timestamp`, `read_at` and position are untouched. |
| `message ack` of a message never shown | Creates the event with `state` `acknowledged`, `read_at` null, `acknowledged_at` and `timestamp` set to the commit time, `revision` 1. |
| First `message show` after such an ack | Sets `read_at` only; `state`, `timestamp` and position are untouched; `revision` advances by one. |
| Replayed or repeated ack, `inbox`, `wait`, `reminder`, refused or failed reads | No change. |

`message inbox` lists metadata only and records nothing. A refusal (authority,
expiry, revision conflict) records nothing. The event is written after the
mailbox registry commit, never before it. A show of an already-read message whose
event is absent, for example after a crash between the two writes, creates the
event with the current time, so the pair converges. An event is never removed by
mail expiry, compaction or session deletion; it belongs to the conversation.

The event belongs to the provider conversation bound to the recipient session at
the moment of the read. A later conversation rebind does not move it.

## Wire shape

Events are elements of `messages` in the existing
`GET /history/sessions/{history_id}/messages` response, in every direction and
with or without `parts=v1`. They add no route, query parameter or cursor form.

```json
{
  "id": "h-<64 hex>-<message_id>-mailbox",
  "role": "notice",
  "text": "<!-- agent-notice:v1 kind=mailbox.read source=agent-session id=<message_id> -->\n\n<body>",
  "timestamp": "2026-11-02T01:02:03.456Z",
  "mailbox": {
    "schema_version": "agent-session.mailbox-read-event.v1",
    "message_id": "<message_id>",
    "sender": {"kind": "session", "machine": "<machine>", "session_id": "<id>"},
    "category": "handoff",
    "state": "read",
    "read_at": "2026-11-02T01:02:03.456Z",
    "acknowledged_at": null,
    "revision": 1,
    "availability": "complete"
  }
}
```

With `parts=v1` the element also carries `"parts": []`, like other machine
notices: there is no structured duplicate body.

| Field | Meaning |
| --- | --- |
| `id` | `<history_id>-<message_id>-mailbox`. Stable for the life of the event. The final segment is never numeric, so a parser of the row-offset suffix of other ids cannot mistake an event for a transcript row. |
| `role` | Always `notice`. The `mailbox` object, not the role alone, identifies this event. |
| `text` | The complete canonical envelope: kind `mailbox.read`, source `agent-session`, id = `message_id`, then the stored message body byte for byte. Header at most 512 bytes, id at most 96, whole text under 64 KiB. |
| `timestamp` | Event creation time. Immutable. UTC RFC 3339 with millisecond precision and `Z`. |
| `mailbox.message_id` | The stored message id. |
| `mailbox.sender` | Display-safe sender: `{kind:"session", machine, session_id}` or `{kind:"service", machine, service_id}`. Incarnation, generation, capability and path values are never present. |
| `mailbox.category` | One of `uncategorized`, `progress`, `handoff`, `blocker`, `decision`, `report`. |
| `mailbox.state` | `read` or `acknowledged`. Never `unread`: an unread message has no event. |
| `mailbox.read_at` | First successful show, or null while the message was only acknowledged. Never changes once set. |
| `mailbox.acknowledged_at` | First acknowledgement, or null. Never changes once set. |
| `mailbox.revision` | 1 at creation, plus one per metadata change (at most 3). A consumer replaces a held event with the same `id` when it sees a higher revision, and ignores a lower or equal one. |
| `mailbox.availability` | Always `complete` in v1: `text` holds the whole original body. |

The body is peer-supplied text. It is data to display, not instruction, and the
marker proves nothing about authority.

## Original body and truncation

Mailbox bodies are at most 16 KiB UTF-8, so a complete envelope is always far
below the 64 KiB history bound. The producer never truncates an event, never
emits a preview, and has no separate full-original route. The authenticated
history messages route is the only body-read surface, and the event it returns
already carries the original text. A producer that ever needed to truncate would
have to add a new `availability` value and an authorized original-body route
through a new schema version; v1 consumers need neither.

A message whose body cannot form a valid envelope (it begins with
`<!-- agent-notice:`, which the encoder refuses as a double envelope, or its id
fails the header grammar) gets no event. It is never escaped or repaired.

## Ordering and paging anchor

History cursors and row ids are physical byte offsets into the provider's
primary transcript. An event uses the same coordinate. When a read or ack
commits, the producer records the transcript's byte length at that moment as the
event's **anchor**, from the exact transcript of the bound provider conversation.
The anchor is an observed number. It is not a provider row id, a provider
timestamp, a guess from adjacent text, or anything found in tool output, so it is
independent of how a harness renders a tool call.

Total order, ascending: anchor `X` for an event and start offset `o` for a row;
an event sorts before a row when `X <= o`, so it follows every row that starts
before `X`. Events with equal anchors keep creation order. The anchor is
assigned at creation and never moves, including on ack.

A page covers a raw byte interval of the transcript. Forward pages cover
`[cursor, next_cursor)`, reverse pages `[older_cursor_new, older_cursor_old)`.
A page contains exactly the events with anchor in `(lo, hi]`, where:

- `lo` is the interval start, and `lo = 0` includes anchor 0 (`[0, hi]`);
- `hi` is the interval end, and a page that reaches the end of the transcript
  (no `next_cursor`, or `direction=latest`) has no upper bound, so an anchor past
  the current length lands on that tail page.

These intervals partition all anchors, so forward, latest and older reads
produce one consistent sequence, and no event repeats or disappears across
pages. Cursors keep their meaning and form. `limit` bounds provider rows only;
events in the page's interval are in addition to it.

If the producer cannot resolve and read the exact transcript when the read
commits, it records no event. It does not place the event by time or at the
tail later. The mailbox read itself is unaffected.

## Authorization

Recording requires the existing authenticated recipient (session capability)
that already performs `message show` or `message ack` for the exact recipient
incarnation. Nothing else can create or update an event.

Reading an event requires exactly what reading the conversation requires: the
server bearer on the history messages route. A missing or invalid bearer gets
the route's existing 401 (503 when no token is configured) and no event text,
metadata or count. A session capability alone is not accepted. An event for one
conversation is never returned for another history id. There is no server-side
switch; Console owns the per-device collapsed or expanded preference.

## Isolation and degradation

Recording never calls notice input, notice delivery, prompt submission, the
notification controller or any provider transport, and never changes a
notification generation. Reading mail therefore adds no agent turn.

The event store is separate from the mailbox registry. If it is missing, locked,
full or unwritable, `message show` and `message ack` still succeed with
unchanged output and no event is recorded. If it is unreadable when history is
served, the route reports its bounded history-read error rather than silently
dropping bound events.

Consumers exclude events from outbox acknowledgement, last-user previews,
stuck or pending reconciliation, unread or attention signals, automatic title
input and incremental semantic-memory selection, exactly as for other machine
notices.

## Fixture corpus

`mailbox-read-events-v1.json` has two groups. `events` holds exact wire
elements: a read event, an acknowledged one, an ack-before-read one, a service
sender and a `parts=v1` form. Every `text` parses with the notice codec.
`placement` holds transcript row offsets, event anchors and page intervals with
the expected forward, latest and older sequences.
