# Saved stale drafts v1

Part of [Serve prompt submission](serve-prompt-v1.md) and [Send v1](send-v1.md).

## Setting and observation

`stale_draft_seconds` in the serve config and
`AGENT_SESSION_STALE_DRAFT_SECONDS` set the unchanged-text bound in seconds.
The environment takes precedence. The default is 600 seconds; 0 disables the
fallback and preserves the nonempty-composer refusal. Values must be unsigned
integer seconds. This setting introduces no aliases or other recovery modes.

Only pending guarded delivery observes age. The first exact nonempty composer
observation starts the timer; the same observed text in the same runtime and
provider session retains it. An edited, empty, or unobservable composer resets
it. A daemon restart retains the observation. Time is measured from these
observations, not terminal activity or the session's waiting status. A pane the
existing reader cannot observe exactly is never eligible for stashing.

The reader's existing limits remain: it may reject multiline or ambiguous panes,
and Claude layout padding is excluded from its observation. The fallback does
not infer hidden text; a cleanup that cannot prove emptiness refuses delivery.

Once the observed text has remained unchanged longer than the bound, admission
persists the exact observed draft before sending any cleanup key. It reuses the
existing draft-cleanup key path, with exact matching and no prefix removal. Counted cursor navigation verifies
the logical end even when the caret was abandoned in the middle of the draft.
Delivery proceeds only after an empty composer is confirmed. The draft is never
submitted, and pending text is never appended to it. Identity, activity,
attachment, coordination, and post-paste guards retain their existing order.
Saving or cleanup failure refuses delivery and leaves the saved text available.
Terminal observations and keystrokes are not atomic against human editing.

## Durable store and access

The store is `<state-dir>/sessions/<session-id>/drafts/`. Its private directories
use mode 0700 and files use 0600, with the same OS owner as the session state.
Only the same operator principals trusted to read the session may read drafts;
HTTP list and read routes require the existing daemon bearer token. Draft text
never appears in board projections, diagnostic messages, or lifecycle logs.

`observation.json` holds a text digest, unchanged-since timestamp, and runtime
and provider-session identity. `saved/<draft-id>.json` is a durable event record
with schema `agent-session.saved-draft.v1`, event `stale_draft_stashed`, its draft
ID, timestamp, runtime identity, exact UTF-8 text, and `cleared` boolean. The
initial record and `latest.json` event metadata are synced before cleanup; `cleared` becomes true only after the
composer is confirmed empty. A crash with false does not imply any text was
removed. Every record names the saved draft; records are retained with the
session and are not silently evicted.

Each draft may contain at most 64 KiB of UTF-8 text. A session holds at most
128 saved drafts and 8 MiB of serialized saved records. Reaching either store
bound refuses fallback without sending keys. Draft IDs are opaque UUIDs.

- `GET /sessions/{id}/drafts` returns `{ drafts: [...] }` in the usual serve
  envelope. Each entry contains `draft_id`, `saved_at` (the save timestamp),
  `session_id` (the owning session), and `bytes` (UTF-8 text byte length), plus
  `event`, `session_incarnation`, and `cleared`. Entries omit text.
  Public metadata uses `saved_at`; `timestamp` remains internal to saved records.
- `GET /sessions/{id}/drafts/{draft_id}` returns `text/plain; charset=utf-8`.
  The response body is the original observed text byte-for-byte, including
  leading/trailing spaces and Unicode, without a newline added by the server.

These routes are always installed; their absence means saved-draft retrieval is
unsupported. There is no feature flag. Setting the fallback bound to 0 leaves
list and read access available.

Console can list these records and restore the raw read body into its editor.
Reading or restoring locally does not submit input, mutate the terminal, or
consume a saved draft. An explicit later prompt submission uses ordinary
admission. No server endpoint automatically replays a saved draft.

## Unresponsive pane observation

`unresponsive_seconds` in the serve config and
`AGENT_SESSION_UNRESPONSIVE_SECONDS` set the response bound in positive integer
seconds. The environment takes precedence. The default is 4 seconds, matching
the existing submission observation budget: it accommodates the 1.5-second
Enter observation/retry interval and the 2.5-second cleanup budget. Zero is
invalid; there are no aliases or additional recovery settings.

Guarded terminal submission and shared composer cleanup record an observation
before their existing key action. Only a recognized, nonempty composer in an
idle provider pane starts an observation. Runtime and provider identity fence
the observation. Private state stores pane digests, never composer text.
Repeated keys on the same unchanged pane retain the original start time.

On subsequent activity reads, an exactly observable pane that has not changed
within the bound projects `turn_state.phase = "unresponsive"`, with observed
terminal-heuristic confidence. The underlying provider activity is preserved.
A provider turn that is busy, pending provider ingress, an unobservable pane,
or changed runtime discards the observation and retains normal activity.
The next observed pane change discards it and restores normal activity without
a restart. This observation introduces no key retry, restart or other recovery;
coordinators choose any recovery action. Existing delivery refusals and submit
outcomes remain in force.

The board's optional `pane_response` field carries the latest metadata-only
event, schema `agent-session.pane-response.v1`: `event` is
`pane_unresponsive`; `operation` is `submit` or `clear`; `session_id`,
`session_incarnation`, `started_at`, `observed_at`, and `bound_seconds` identify
the observation. The latest event remains available after the status clears.
Neither pane bytes, composer text, nor private digests enter the event.
