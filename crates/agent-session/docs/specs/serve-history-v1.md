# Serve history and archive v1

Part of [Serve API v1](serve-api-v1.md). Its scope and shared rules also apply here.

## History operations

### Provider session history

`GET /history/sessions` returns a bounded, cursor-paged catalog of Codex and Claude provider
sessions. DSH launch profiles are retired, so serve does not configure a DSH history source. Optional `q`
searches only metadata already present in the catalog snapshot: exact archived/managed title, bounded
first-user-prompt preview when the provider scan supplies one, provider session id, provider,
launch-profile id, cwd/repository label, machine, and timestamps.

The query never searches arbitrary conversation text or the latest-user-prompt preview. Optional
`provider`, `cursor`, and `limit` further bound the
result. Each item may add `first_user_prompt_preview` and `last_user_prompt_preview`; both are bounded
response-only text and may be absent when provider-aware parsing cannot identify a human-submitted prompt
within the read budget. `prompt_preview` remains the backward-compatible alias for the first-user
preview.

A matching managed record enriches `title` only through the exact provider/profile/session history
identity; archived metadata stays authoritative, and provider-only historical records do not invent a
Console title. `data.capabilities` explicitly advertises `metadata_search: true`, `full_text_search:
false`, `transcript_messages: true`, `latest_message_paging: true`, `archive: true`, and `star: true`,
and `resume: true`.

Starred sessions lead the page, newest star first, and the unstarred remainder
follows in the retained recency order. Each item may add `starred_at`. Cursors
stay opaque: the current form is `v2` and carries the star, while a `v1` cursor
minted before stars existed is still accepted and resumes as unstarred.

Latest-prompt enrichment is limited to the returned catalog page, uses at most
1 MiB per transcript and 16 MiB across one page, and is cached in process by
transcript path, size, and modification time. It never adds a full-catalog
transcript-body scan.

Retained DSH provider history is not listed or resumed by the serve API. Remove
legacy DSH launch profiles from the daemon configuration.

`GET /history/sessions/{history_id}/messages` resolves the opaque history id inside the daemon and
returns normalized `user`/`assistant` text messages and Codex `goal` creation objectives in bounded
pages. Injected AGENTS and Goal continuation context are excluded. Claude user messages include prompts
the user sent while a turn was running (`queued_command` attachments); queued task notifications, peer
messages, auto-continuations, meta or machine-markup prompts, and agent-session mailbox reminders are
excluded. A slash command that Claude Code records as command markup is shown as typed (`/name args`),
once; local-command output and interrupt markers recorded as `user` rows are excluded.

Queue bookkeeping (`queue-operation`) never adds a message, so a queued prompt appears once. The retained
default `direction=forward` uses `next_cursor` as before. `direction=latest` accepts no cursor, reads one
at-most-16-MiB window from the selected transcript tail, and returns the latest page in chronological
order. `direction=older` requires the prior `older_cursor`, reads only bytes before that line boundary,
and returns the preceding chronological page plus a new optional `older_cursor`. Reverse reads retain the
10,000-line and two-second message limits. The browser receives neither provider transcript paths nor raw
provider JSONL records.

Both history reads require the server bearer because conversation content is more sensitive than the live
list projection.

Clients may opt into typed transcript parts with `?parts=v1` on the messages route. Each message then
adds an ordered `parts` array, including an empty array when no typed part is available. Parts include
visible `text`, `tool_call`, `tool_result`, `image`, `question`, and recognized `notice` records. Every
part carries the provider item `id` and its timestamp when available. Tool input and result summaries
are capped at 2,048 characters and redact path-like values and sensitive input fields. Typed output is
capped at 64 KiB per message. `tool_call.status` is `unknown` when a matching result is not found in
the returned page or the following 1 MiB transcript context; a result sets `completed` or `failed`,
with `started_at` and `ended_at` when provider timestamps are available. Image `ref` values are opaque ids for the authenticated image route;
`mime` and `bytes` may be absent when the source does not safely expose them. The `parts` query value
is versioned and unknown values are rejected. Without it, the existing response shape is unchanged.

Claude Code typed parts are projected only from transcript records tagged with version `2.1.x`; records
with missing or other versions remain available through the existing text and image projection. Codex
typed parts are projected only when the session metadata records a `0.x.y` CLI version, and are gated
by recognized `response_item` and `event_msg` record shapes. Missing or other Codex versions and
unrecognized record types are omitted. The `history-parts` fixtures pin the supported Claude Code
`2.1.x` and Codex `0.x.y` ranges.

Message pages also carry bounded optional image descriptors, including image-only
messages. [History images v1](history-images-v1.md) owns the authenticated binary
read route, source/path boundaries, byte limits and deleted/unavailable outcomes.

`POST /history/sessions/{history_id}/star` sets or clears one star, keyed by the
history id the daemon itself minted: the body is `{ "starred": <bool> }`, the id
is resolved against the catalog first, and an id that names no history session is
a not-found rather than a stored record. The response carries `history_id` and
`starred_at`, which is null once the star is cleared. Clearing a star that was
never set is a success, so a repeated toggle from two clients settles.

`POST /history/sessions/{history_id}/resume` accepts no body. The daemon scans
the owning provider source again, recomputes the opaque history identity, and
derives provider id, cwd, profile, executable, and canonical resume arguments
only from trusted server-side metadata. Codex and Claude use their existing
provider-import behavior. DSH history resume returns
`history-resume-not-supported` and does not create a session.

A conversation keeps one managed identity across archive and resume. Archive
records the managed session id and structured title state beside the title.
History resume reuses that archived session id when no session directory holds
it, so peers that address the conversation by id keep reaching it, and restores
the archived title and title state. An archive written before the session id
was recorded, or an id another session now holds, falls back to a newly
allocated id. When a live managed record already owns the conversation, resume
launches nothing and returns `history-session-live` with that session's `id` in
the error details, so one conversation never runs under two session ids.

The history-resume boundary uses this bounded status mapping: 404
`history-session-not-found`; 409 `provider-session-already-running` or
`history-session-live`; 422
`history-resume-not-supported` or `history-resume-identity-mismatch`; 503
`history-resume-profile-unavailable`; and 500
`history-read-failed`. Errors raised after trusted launch admission retain the
existing managed-session startup error contract.

`POST /sessions/{id}/archive` requires `expected_session_incarnation` and accepts an optional `starred`
boolean, which stars the session as it lands in history and is reflected as `archived.starred_at`. Older
clients omit it. The archive itself is already committed when the star is written, so a star that cannot
be stored is reported as unstarred rather than failing the completed archive. It writes private,
mode-0600 Console metadata for the captured provider session and then runs the existing verified session
deletion path. A changed incarnation returns a conflict. If verified deletion fails, the new archive
metadata is rolled back and the live record remains retryable.

Archive never deletes or rewrites the provider transcript. Existing `DELETE /sessions/{id}` remains the
distinct permanent Console-record removal operation and likewise does not delete provider history.

Both routes refuse a session that sessions on this machine still name as their
effective parent: HTTP 409 `session-has-live-children`, with
`details.children` and `details.scope`, before anything is archived or
deleted. The archive body's optional `orphan_children: true`, or
`DELETE /sessions/{id}?orphan_children=true`, closes it anyway; any other query
key or value fails with HTTP 400 `invalid-query`. On success
`deleted.children` reports `{scope, orphaned}`. See
[Session lineage and work v1](session-lineage-work-v1.md#closing-a-parent).

`GET /sessions` advertises additive `history`, `archive`, `group_archive`, and
`history_star` capabilities. Older daemons omit them and do not serve these
routes.
