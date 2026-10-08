# Serve API v1

This specification owns the current HTTP and WebSocket surface exposed by
`agent-session serve`. The crate README is a non-normative product entrypoint;
the operations runbook owns deployment procedure.

## Route ownership

This index covers every route literal registered by the daemon. Braced
comma-separated route segments below are exact alternatives, not wildcards.
`Bearer + capability` means the server bearer plus
`X-Agent-Session-Capability`.

| Method and path | Authentication | Canonical contract |
| --- | --- | --- |
| `GET /healthz` | Open | This specification |
| `GET /board/v1` | Bearer; only with `serve --board` or `AGENT_SESSION_BOARD=1` | [Session board v1](session-board-v1.md#daemon-local-snapshot) |
| `GET /board/closed/v1` | Bearer; only with `serve --board` or `AGENT_SESSION_BOARD=1` | [Session board v1](session-board-v1.md#closed-session-ledger) |
| `GET /board/programs/v1` | Bearer; only with `serve --board` or `AGENT_SESSION_BOARD=1` | [Session board v1](session-board-v1.md#programs) |
| `GET /shells/{owner}` | Bearer | [Emergency shells](#emergency-shells) |
| `POST /shells/{owner}` | Bearer | [Emergency shells](#emergency-shells) |
| `DELETE /shells/{owner}` | Bearer + incarnation body | [Emergency shells](#emergency-shells) |
| `GET /shells/{owner}/attach?incarnation=...` | Bearer + incarnation query | [Emergency shells](#emergency-shells) |
| `GET /sessions` | Open | This specification |
| `POST /sessions` | Bearer | This specification |
| `GET /history/sessions` | Bearer | This specification |
| `GET /history/sessions/{history_id}/messages` | Bearer | This specification |
| `POST /history/sessions/{history_id}/star` | Bearer | This specification |
| `POST /history/sessions/{history_id}/resume` | Bearer | This specification |
| `GET /codex/accounts` | Bearer | [Codex account broker](#codex-account-broker) |
| `GET /claude/accounts` | Bearer | [Claude account broker](#claude-account-broker) |
| `POST /clipboard/unwrap/v1` | Bearer | This specification |
| `GET /activity/events` | Bearer | [Activity stream v1](activity-stream-v1.md) |
| `POST /activity/hook/v1` | Session capability only; direct loopback peers only | [Activity stream v1](activity-stream-v1.md#provider-hook-ingress) |
| `GET /usage` | Open | This specification |
| `GET /usage/v1` | Bearer | [Provider usage and resets](#provider-usage-and-resets) |
| `POST /codex/reset/v1` | Bearer; account allowlist and idempotency key | [Provider usage and resets](#provider-usage-and-resets) |
| `POST /claude/reset/v1` | Bearer; account allowlist and idempotency key | [Provider usage and resets](#provider-usage-and-resets) |
| `GET /workdirs` | Bearer | This specification |
| `GET /repos/remote-url` | Bearer | This specification |
| `GET /sessions/{id}/glance` | Open | This specification |
| `GET /sessions/{id}/work-context/v1` | Bearer | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `POST /sessions/{id}/work-context/check/v1` and `POST /coordination/work-context/check/v1` | Bearer; session capability optional where supported | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `POST /sessions/{id}/work-context/{claim,renew,release,admit,complete,reconcile}/v1` | Bearer + capability | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `GET /sessions/{id}/broker/v1` | Bearer | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `POST /sessions/{id}/broker/{adopt,reconcile}/v2` | Bearer + capability; recovery proof is in the request body | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `POST /sessions/{id}/broker/{adopt,reconcile}/v1` | Bearer + capability; retained transition alias for the v2 authorization contract | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `POST /sessions/{id}/operations/{lease_id}/operator-reconcile/v1` | Bearer; explicit confirmed operator attestation | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `POST /sessions/{id}/activity/provider-turn/operator-reconcile/v1` | Bearer only; explicit confirmed inactive attestation | [Provider-turn reconciliation](session-coordination-v1.md#operator-provider-turn-reconciliation) |
| `GET /coordination/messages/audit/v1` | Operator bearer | [Mail audit v1](mail-audit-v1.md) |
| `GET and POST /sessions/{id}/messages/v1` | Bearer + capability | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `GET /sessions/{id}/messages/{message_id}/v1` | Bearer + capability | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `POST /sessions/{id}/messages/{message_id}/{ack,reply}/v1` | Bearer + capability | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `GET /sessions/{id}/messages/{message_id}/wait/v1` | Bearer + capability | [Coordination HTTP coverage](session-coordination-v1.md#http-coverage) |
| `GET /sessions/{id}/buffer` | Open | This specification |
| `POST /sessions/{id}/{send,prompt,prompt/v2,resume}` | Bearer | This specification |
| `POST /sessions/{id}/archive` | Bearer | This specification |
| `GET /sessions/{id}/maintenance` and `POST /sessions/{id}/maintenance/actions` | Bearer | [Session maintenance v1](session-maintenance-v1.md#authentication-and-endpoints), successor [v2](session-maintenance-v2.md#negotiation) |
| `GET and POST /sessions/{id}/orchestration/group-cleanup` | Bearer | [Main Agent orchestration v1](main-agent-orchestration-v1.md#daemon-owned-group-cleanup) |
| `GET and POST /sessions/{id}/orchestration/group-archive` | Bearer | [Main Agent orchestration v1](main-agent-orchestration-v1.md#daemon-owned-group-archive) |
| `PUT /sessions/{id}/account` | Bearer | This specification |
| `GET /sessions/{id}/auto-resume` | Open | This specification |
| `PUT and DELETE /sessions/{id}/auto-resume` | Bearer | This specification |
| `POST /sessions/{id}/attachments` | Bearer | This specification |
| `GET /sessions/{id}/attach` | Bearer | This specification |
| `PATCH and DELETE /sessions/{id}` | Bearer | This specification |

## Endpoint contracts

`agent-session serve` exposes the session control plane over loopback HTTP for a per-machine edge (e.g. the agent-console
web console). It builds its own tokio runtime and reuses the synchronous lifecycle functions via `spawn_blocking`, so there
is no second state model.

### Clipboard extraction

`POST /clipboard/unwrap/v1` accepts `{ "selection": string }` with at most
4,096 characters (and 16 KiB of UTF-8), rejecting empty and control-bearing
selections. It shares retitle's bounded provider queue and Codex subscription
account selection, and requires that provider's configured model to be
`gpt-6-luna`. Local and other fallback providers are not used for clipboard
extraction. The response `data` contains the machine name and a strict JSON
model output string. The edge validates that output against the original
selection before copying. A standalone absolute filesystem path is returned
with `kind: path`; URL and shell results keep their existing kinds. The daemon
does not store or log the selection or
model output.

### Provider session history

`GET /history/sessions` returns a bounded, cursor-paged catalog of Codex,
Claude, and DSH provider sessions. DSH entries carry `resumable: true` only
when their owning ready launch profile explicitly advertises exact-ID history
resume; older and read-only profiles keep `resumable: false`. Optional `q`
searches only metadata already present in the
catalog snapshot: exact archived/managed title, bounded first-user-prompt
preview when the provider scan supplies one, provider session id, provider,
launch-profile id, cwd/repository label, machine, and timestamps. DSH's
provider-generated title and prompt previews are selected-page enrichments and
are not query inputs, which preserves the header-only catalog scan. The query
never searches arbitrary conversation text or the latest-user-prompt preview.
Optional `provider`, `cursor`, and
`limit` further bound the result. Each item may add
`first_user_prompt_preview` and `last_user_prompt_preview`; both are bounded
response-only text and may be absent when provider-aware parsing cannot
identify a human-submitted prompt within the read budget. `prompt_preview`
remains the backward-compatible alias for the first-user preview. A matching
managed record enriches `title` only through the exact provider/profile/session
history identity; archived metadata stays authoritative, and provider-only
historical records do not invent a Console title. `data.capabilities` explicitly
advertises `metadata_search: true`, `full_text_search: false`,
`transcript_messages: true`, `latest_message_paging: true`, `archive: true`, and
`star: true`, and `resume: true`.

Starred sessions lead the page, newest star first, and the unstarred remainder
follows in the retained recency order. Each item may add `starred_at`. Cursors
stay opaque: the current form is `v2` and carries the star, while a `v1` cursor
minted before stars existed is still accepted and resumes as unstarred.

Latest-prompt enrichment is limited to the returned catalog page, uses at most
1 MiB per transcript and 16 MiB across one page, and is cached in process by
transcript path, size, and modification time. It never adds a full-catalog
transcript-body scan.

DSH history is supplied by a launch-profile-owned external adapter. Catalog
scans invoke its header-only `list` command; only the selected page is passed to
`summaries`, and message reads pass one exact provider session id to `messages`.
The adapter is invoked directly without a shell in a separate process group,
with bounded output and a ten-second deadline. A missing, timed-out, non-zero,
or malformed adapter degrades that history read and does not affect launch
profile readiness or live DSH sessions. The daemon never exposes the adapter
command, session root, native event records, or transcript paths.

`GET /history/sessions/{history_id}/messages` resolves the opaque history id
inside the daemon and returns normalized `user`/`assistant` text messages and
Codex `goal` creation objectives in bounded pages. Injected AGENTS and Goal
continuation context are excluded. Claude user messages include prompts the
user sent while a turn was running (`queued_command` attachments); queued task
notifications, peer messages, auto-continuations, meta or machine-markup
prompts, and agent-session mailbox reminders are excluded. A slash command
that Claude Code records as command markup is shown as typed (`/name args`), once;
local-command output and interrupt markers recorded as `user` rows are excluded. Queue bookkeeping
(`queue-operation`) never adds a message, so a queued prompt appears once. The retained default `direction=forward` uses `next_cursor` as
before. `direction=latest` accepts no cursor, reads one at-most-16-MiB window
from the selected transcript tail, and returns the latest page in chronological
order. `direction=older` requires the prior `older_cursor`, reads only bytes
before that line boundary, and returns the preceding chronological page plus a
new optional `older_cursor`. Reverse reads retain the 10,000-line and two-second
message limits. The browser receives neither provider transcript paths nor raw
provider JSONL records. Both history reads require the server bearer because
conversation content is more sensitive than the live list projection.

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
provider-import behavior. DSH requires an owning ready profile with base agent `dsh` whose
`dsh_history.resume` is `exact-id`; it persists provider `dsh` and invokes the
profile with exactly `--resume <provider-session-id>`. The response contains the normal newly
created managed `session` projection.

A conversation keeps one managed identity across archive and resume. Archive
records the managed session id and structured title state beside the title.
History resume reuses that archived session id when no session directory holds
it, so peers that address the conversation by id keep reaching it, and restores
the archived title and title state. An archive written before the session id
was recorded, or an id another session now holds, falls back to a newly
allocated id. When a live managed record already owns the conversation, resume
launches nothing and returns `history-session-live` with that session's `id` in
the error details, so one conversation never runs under two session ids.

For a fresh `POST /sessions` using a server-owned `dsh` profile with
`dsh_history.resume=exact-id`, agent-session allocates the DSH UUID before
creating the managed record. It stores the exact provider identity and canonical
history root in `provider_resume`, then passes
`--agent-session-seed <provider-session-id>` to the trusted profile launcher.
That launcher materializes an empty DSH session through the official persistence
API and starts the unmodified TUI with `--resume <provider-session-id>`.
The coordination broker holds the provider lease before releasing the TUI launch
gate. Missing history storage refuses the new launch; caller-supplied resume or
seed arguments cannot replace the managed identity.

Before the DSH provider process is released from its held launch gate, its
runtime heartbeat sidecar obtains a private filesystem lease keyed by the
canonical history root and provider session identity. The sidecar holds the
kernel lock for the runtime lifetime. A competing launch returns
`provider-session-already-running` with HTTP 409 and never starts a second
provider writer; process exit releases the lock without PID-based stale-owner
cleanup. Missing history returns `history-session-not-found`, an unsupported
profile returns `history-resume-not-supported`, and unavailable provider
history or profile readiness fails without launching a runtime.

The history-resume boundary uses this bounded status mapping: 404
`history-session-not-found`; 409 `provider-session-already-running` or
`history-session-live`; 422
`history-resume-not-supported` or `history-resume-identity-mismatch`; 503
`history-resume-profile-unavailable` or `dsh-history-unavailable`; and 500
`history-read-failed`. Errors raised after trusted launch admission retain the
existing managed-session startup error contract.

`POST /sessions/{id}/archive` requires
`expected_session_incarnation` and accepts an optional `starred` boolean, which
stars the session as it lands in history and is reflected as `archived.starred_at`.
Older clients omit it. The archive itself is already committed when the star is
written, so a star that cannot be stored is reported as unstarred rather than
failing the completed archive. It writes private, mode-0600 Console metadata
for the captured provider session and then runs the existing verified session
deletion path. A changed incarnation returns a conflict. If verified deletion
fails, the new archive metadata is rolled back and the live record remains
retryable. Archive never deletes or rewrites the provider transcript. Existing
`DELETE /sessions/{id}` remains the distinct permanent Console-record removal
operation and likewise does not delete provider history.

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

### Operator provider-turn reconciliation

`POST /sessions/{id}/activity/provider-turn/operator-reconcile/v1` is the
Bearer-only server-operator repair for one exact open provider turn whose
authoritative completion signal is missing. `X-Agent-Session-Capability` is
ignored as authority; a target-session capability without the server Bearer
returns `401 unauthorized` before body processing or mutation.

The route uses the ordinary serve success/error envelope with its result under
`data.coordination`. `activity-revision-conflict` is an HTTP `409`; malformed
requests are `400`, unavailable dependencies are `503`, and other rejected
admission evidence follows the shared coordination error mapping. Request and
result schemas, exact admission fences, preservation, receipt TTL/quota,
idempotency, and stable failure codes are normative in
[Session Coordination V1](session-coordination-v1.md#operator-provider-turn-reconciliation).

`GET /healthz` additively reports the distinguishable health states a console
needs. `data.status` keeps its historical meaning — this handler answered — so an
existing consumer is unaffected. Beyond that:

- `data.health` is `healthy`, `degraded`, or `critical`.
- `data.runtime.state` is `available`, `degraded`, or `unavailable`, with
  `data.runtime.reasons` carrying stable codes.
- `data.runtime.executable_state` is `live`, `replaced`, or `unknown` from
  `/proc/<pid>/exe`. A daemon answering from a replaced executable is
  `unavailable`: an upgrade can leave the installation symlink correct while the
  live process holds a deleted inode.
- `data.runtime.coordination` is `available`, `absent`, or the stable read
  failure, where a registry from another release generation reports
  `runtime-version-skew` rather than `coordination-invalid`.
- `data.sessions.protected` counts sessions whose broker is ready with a fresh
  heartbeat. Their runtimes must not be restarted out from under an active claim
  or an uncertain operation.

`machine offline`, `runtime unavailable`, and `session protected` are three
different operator situations with three different responses. Machine
reachability is answered by receiving a response at all; the other two are the
fields above. Collapsing them into one generic offline state is the reporting gap
recorded in `sympoies/nils-cli#1409`.

- `GET /healthz`, `GET /sessions`, `GET /sessions/{id}/glance?tail=N` — reads, open on loopback. `GET /sessions`
  additively reports `data.observed_at`, sampled from daemon time after the returned session state is assembled, plus
  `data.agent_profiles` containing only ready server-owned
  `{ id, label, agent, provider_resume_import_supported, history_resume_supported }` launch-profile
  summaries. `data.capabilities.profile_resume_import` advertises support for
  selecting one of those safe ids during provider import; executable paths,
  configuration roots, readiness commands, and environment remain private.
  `data.capabilities.managed_resume_command` remains false until an unqualified
  CLI resume can revalidate the daemon's active profile registry and readiness
  contract; consumers must copy the provider session ID during that skew.
  `data.capabilities.managed_account_handoff` advertises that this daemon
  understands the managed-handoff protocol; it does not make every session
  eligible. A session-level `capabilities` array contains
  `agent-session.codex-managed-account-handoff.v1` only for a live Codex
  app-server runtime whose launch-bound capability probe succeeded. Raw Codex
  tmux sessions and all Claude sessions omit that session capability. Consumers
  must require both the global protocol advertisement and the per-session
  capability before presenting managed handoff controls.
  `data.capabilities.codex_account_switch` and
  `data.capabilities.claude_account_switch` are each `true` only while that
  provider's broker (`AGENT_SESSION_CODEX_ACCOUNT_BROKER` /
  `AGENT_SESSION_CLAUDE_ACCOUNT_BROKER`) is configured; see
  [Account brokers](#account-brokers). `codex_account_switch` is additive:
  older daemons omit it, and a Codex session's own
  `codex_account.supported` remains the per-session authority.
  Sessions report
  `running`, `stopped`, `unknown`, or `missing` live status plus a boolean `resumable` field and best-effort `repo_name` derived from
  the recorded `cwd`. `missing` is reported only for an external-runtime record
  (`runtime.kind = "dsh_external"`) whose owning plugin never attached to the
  recorded launch; consumers must treat it as "no runtime exists for this
  launch", never as a terminated runtime that could be resumed. Every
  external-runtime record reports `resumable: false` — it carries no provider
  resume identity — and its runtime is owned by the external plugin, so the
  input path refuses it with `dsh-runtime-plugin-owned`. Resume and input
  controls belong to that plugin, not to this daemon. New interactive records also expose optional
  `runtime_started_at`, `turn_state`, `last_prompt`, `last_prompt_state`,
  `last_prompt_continuity`, and `startup`; a profiled
  session also
  exposes its safe `agent_profile` id. When a known profile drift would make a
  stopped session fail resume, the daemon sets `resumable: false` plus one
  bounded `resume_blocked_reason` code. Old records and daemons omit the
  additive fields.
  `data.capabilities.last_prompt` advertises the list `last_prompt` preview: the
  most recent user prompt for a running Codex/Claude session, resolved from the
  exact provider transcript so it reflects prompts submitted through any input
  path (web console, SSH/Termius, or raw `tmux attach`). Codex Goal creation
  and objective edits also update this preview as `Goal: <objective>` in
  transcript order; later user messages replace it. Goal status/usage updates
  and internal continuation context do not revive an older instruction. Goal
  observation is preview-only and never emits `prompt_submitted` or acknowledges
  a broker input delivery. Inline string-valued `image_url` content is elided
  before the 256 KiB line-buffer bound, preserving surrounding user text without
  retaining image payloads in that buffer. For Codex records containing an actual
  image, paired image name/path wrapper lines are removed from the list preview
  so they cannot hide the user question; submission-event text stays unchanged. If cold recovery starts mid-history, the first
  existing Goal snapshot establishes its comparison baseline without replacing
  a newer prompt, unless its event timestamp identifies a new Goal creation.
  An objective edit cannot be inferred from an isolated existing snapshot;
  subsequent objective changes are observed normally.
  The 64 KiB per-read and 16-read catch-up budgets
  remain in force; oversized non-image records still fail closed.
  On first discovery the
  daemon opens an append tail and queues one at-most-64-MiB cold recovery outside
  the list-response path. Cold recovery and append catch-up are single-flight per
  session and share a daemon-wide concurrency bound. Eligible running
  Codex/Claude sessions add `last_prompt_state` with one of `current`, `pending`,
  or `unavailable`. `current` may include `last_prompt`; `current` without it
  authoritatively means the caught-up transcript has no eligible user prompt.
  A managed Main Agent worker does not preview its launch prompt once
  `main-agent` has bound it to its assignment (`orchestration.role` is
  `worker`). A prompt identical to the session's own private launch prompt is
  withheld as `current` without `last_prompt` until a later prompt replaces
  it. That prompt is the controller-generated bootstrap instruction and names
  a machine-local executable.
  `pending` means exact-source discovery, cold recovery, or known append catch-up
  is in progress and omits the preview rather than reporting a stale cached
  value. `unavailable` means the exact source cannot currently be used or its
  continuity was invalidated, and also omits the preview. After continuity
  invalidation, every caller continues to see `unavailable` until one response
  can expose an authoritative `current` projection. The response-only opaque
  `last_prompt_continuity` token is 16-128 URL-safe ASCII characters, is present
  with eligible states, and rotates whenever exact transcript continuity is
  lost, including across daemon restarts. Consumers may retain a pending
  preview only when this token and the runtime identity both match. Sessions
  without
  enough runtime/provider identity to be eligible omit both fields. Later list
  polls use a bounded metadata/continuity check, retain the newest caught-up
  preview in process memory, and avoid recurring cold scans when the stable
  registry is at capacity. Rotation, truncation, or identity drift invalidates
  that memory and forces exact rediscovery. The text is returned in the response
  only and is never logged or persisted by the daemon; the daemon never guesses
  a transcript match.
  `startup` is the metadata-only `agent-session.startup.v1` projection shared by
  create, list, and glance responses. Its state is `starting`, `ready`, or
  `failed`; its bounded stage is `record`, `tmux`, `runtime`, `app_server`,
  `proxy`, `provider_client`, or `initial_connection`.
  A Codex session whose automatic runtime selection kept the raw TUI adds
  `runtime_fallback` with one allowlisted reason: `codex-unavailable`,
  `codex-version-unrecognized`, `codex-version-too-old`,
  `codex-app-server-transport-unavailable`,
  `codex-app-server-runtime-dir-unavailable`,
  `codex-app-server-runtime-dir-unsafe`,
  `codex-app-server-socket-path-too-long`, or
  `codex-app-server-runtime-unavailable`. It is absent for an app-server
  runtime and for an explicit `AGENT_SESSION_CODEX_RUNTIME=raw`.
  Failed projections add
  an RFC 3339 `occurred_at` captured from the private failure marker, boolean
  `retry_safe`, one reviewed message, and one
  allowlisted code: `runtime-helper-unavailable`, `agent-binary-unavailable`,
  `working-directory-unavailable`, `terminal-runtime-create-failed`,
  `app-server-start-failed`, `proxy-start-failed`, `provider-client-exited`,
  `provider-configuration-rejected`, `startup-timeout`, `startup-exited`, or
  `claude-account-switch-resume-failed`, or `claude-account-switch-cleanup-incomplete`.
  A failure whose cleanup could not finish adds a bounded `cleanup` object with
  `state` of `pending` or `blocked` and one allowlisted `reason`:
  `session_still_running`, `process_boundary_live`, `runtime_identity_changed`,
  `runtime_identity_unavailable`, `termination_failed`, `termination_timeout`,
  `verification_failed`, `cleanup_unavailable`, or `unknown`. `cleanup`
  is absent when cleanup completed or was never attempted, so a projection that
  omits it carries no caveat. Cleanup is strictly secondary: it never replaces
  the primary startup failure, because the original create/start error is what
  explains the failure and a termination error would hide it. When cleanup does
  not complete, the session record is deliberately retained rather than removed —
  a live boundary with no record would be unreachable from the Console — and the
  runtime is not marked never-launched.
  Managed launchers retain only bounded stage/failure markers in the record and
  keep stderr in a private, tail-capped local diagnostic file for startup failures
  and non-zero Codex provider-client exits after readiness; clean exits discard it.
  The local `agent-session logs <id>` command can read that diagnostic, but it is
  never copied into the session projection. Raw argv, environment, provider responses,
  stderr, prompts, and filesystem paths are never copied into the projection. A
  record that reached `ready` keeps that state after an ordinary later stop,
  so consumers must not relabel normal session termination as startup failure.
  A fresh managed Codex client that exits nonzero before binding its first
  provider thread reports `provider-client-exited`, even if an earlier view
  reported `ready` from the initial proxy connection. A resume starts a fresh startup
  lifecycle for its new runtime generation; synchronous launch rollback restores
  the prior projection and private diagnostic artifacts. A leftover resume
  backup from an interrupted process blocks another resume before mutation so
  the only copy of prior diagnostic state is not silently discarded.
- `GET /usage` — read-only provider usage report, open on loopback. The serve
  envelope contains `data.usage.schema_version: "agent-session.usage.v1"` and
  provider entries for Codex and Claude. Provider readers are bounded by
  `AGENT_SESSION_USAGE_TIMEOUT_MS` (default 45000). The Claude reader forwards
  that budget to its nested probe with five seconds reserved before the outer
  hard deadline when the budget is at least six seconds; shorter budgets use a
  one-second inner minimum. A positive
  `CLAUDE_PROMPT_SEGMENT_CLAUDE_TIMEOUT_SECONDS` override is kept when it fits
  and clamped when it exceeds that inner budget. Timed-out helpers are killed as
  a process group before their output pipes are read. Provider readers preserve
  partial success, preserve reset timestamps as `reset_at_epoch` epoch seconds
  plus textual `reset_at` when supplied by the helper, and redact tokens, local
  auth paths, and private account identifiers from scoped error messages.
  Failed providers may include the additive provider-neutral `reason_code`
  contract (`auth_required`, `auth_expired`, `billing_past_due`,
  `subscription_inactive`, `organization_disabled`, `permission_denied`,
  `rate_limited`, `service_unavailable`, `timeout`, or `unknown`) copied only
  from the helpers' allowlisted structured field. The authenticated, cached,
  per-account successor is `GET /usage/v1`, specified under
  [Provider usage and resets](#provider-usage-and-resets).
- `GET /repos/remote-url?cwd=...` — authenticated repository lookup. `cwd` is
  required. The ordinary serve envelope returns `data.url` as a normalized
  credential-free HTTPS URL for any parseable Git origin host, or `null` when
  the directory has no supported remote.
- `GET /sessions/{id}/buffer` — open on loopback. Returns the tmux server's
  latest global clipboard buffer after using `id` only to verify that the
  requested session exists. The buffer is not scoped to that session.
- `POST /sessions/{id}/messages/v1` and
  `POST /sessions/{id}/messages/{message_id}/reply/v1` persist unread mail and
  schedule an eventual body-free mailbox notification. Their coordination
  result adds `{ state, generation, notified_generation, last_reason?,
  controller_available }` under `notification`; the active server sets
  `controller_available: true`. Delivery is controller-owned and may occur
  later when the exact managed Codex or Claude incarnation reaches its fenced
  safe-input boundary. The response never implies that the message was read,
  and it exposes no mailbox body, receipt key, capability, incarnation, or
  provider turn identifier. The full state, adapter, and privacy contract is in
  [Session coordination v1](session-coordination-v1.md#notification-ownership).
  Codex app-server runtimes use acknowledged structured submission: idle turns
  use `turn/start`, while an authoritative active turn uses `turn/steer` with
  its exact `expectedTurnId`. The control connection transiently maps the
  durable projected turn fence back to the matching raw active turn id; raw
  ids never enter session documents or API projections. Long-running work observes the body-free prompt
  at the provider's next model checkpoint. Both paths retain the exact
  incarnation and no-claim/no-operation generation fence.
  Terminal-backed Codex and Claude runtimes use the same notification-generation CAS
  plus exact incarnation, idle-turn, live-runtime, and detached-session
  rechecks before one fixed body-free prompt and a single Enter. The terminal
  attempt boundary additionally requires an authoritative broker with no
  active claim or active/uncertain operation. Its durable per-session
  `attempting` state fences new claim and operation admission through
  submission without holding the global coordination registry lock across
  terminal I/O. Transcript observation determines acknowledged versus unknown
  outcome.
- Every session view additively includes `auto_resume` using
  `agent-session.auto-resume.v1`. `GET /sessions/{id}/auto-resume` reads that
  status and is open on loopback; `PUT` with `{ "enabled": true|false,
  "recovery_policy": "wait_for_reset"|"next_account_then_resume" }` opts a
  supported session in or out, and `DELETE` durably cancels pending work. The
  policy defaults to `wait_for_reset`. Account failover is accepted only for a
  bound, broker-backed Codex session. It selects the next configured account
  with fresh confirmed capacity, applies that account through external auth,
  and only then enters the existing exactly-once continuation claim. The
  structured rejection remains authoritative when the current percentage
  windows are still open, because workspace-credit exhaustion is independent
  from those windows. Exhausted and already-attempted accounts are not revisited
  in the same recovery chain, including when a failed automatic continuation
  produces a new provider turn; a later manual input starts a fresh chain. When
  an explicit account switch wins the race with the rejected turn, the input
  fence identifies the older account and binding revision, and the daemon may
  continue once a different newly bound account reports authoritative open
  usage without first invoking the broker again. Pre-upgrade recovery state that
  lacks either part of that input-binding identity fails closed with bounded
  `state_unavailable` retries: it does not invoke the broker, queue an account
  switch, or submit a continuation. When no candidate is
  trustworthy, a current exhausted percentage
  window waits for its confirmed reset; an open current window reports
  `no_account_available` after bounded discovery retries instead of mislabeling
  the provider rejection as `usage_window_not_exhausted`. Both
  mutations require the bearer token. Claude Code is supported through its
  authoritative structured
  `StopFailure.error == "rate_limit"` signal. Fresh interactive Codex sessions
  created through the serve API are also supported when the installed CLI
  capability probe selects the app-server v2 runtime. The daemon consumes the
  live metadata-only protocol
  and requires an exact bound thread/turn with terminal `status == "failed"`
  plus `codexErrorInfo == "usageLimitExceeded"` or the internal
  `serverOverloaded` capacity cause. Capacity recovery preserves the public v1
  projection, waits 30, 60, 120, 300, and 600 seconds across at most five
  submitted attempts, and uses this fixed control-channel message:
  `The selected model was at capacity, interrupting the previous turn. Please
  continue from where you stopped.` Standalone/raw Codex TUI,
  imported Codex conversations, and resumed pre-app-server Codex sessions remain
  unsupported. Terminal text and assistant output are never treated as
  authority.
  The response object is `{ schema_version, supported, enabled, recovery_policy,
  state,
  scheduled_at?, failure_reason? }`. `scheduled_at`, when present, is an
  RFC 3339 timestamp. The v1 `state` values are `disabled`, `enabled`, `armed`,
  `scheduled`, `switching_account`, `checking`, `resumed`, `cancelled`, `transient_failure`, and
  `terminal_failure`. The allowlisted `failure_reason` values are
  `state_unavailable`, `manual_input`, `usage_unavailable`,
  `usage_window_not_exhausted`, `exhausted_reset_unavailable`,
  `session_state_changed`, `submission_outcome_unknown`, `provider_unsupported`,
  `account_switch`, `no_account_available`, `account_switch_failed`, and
  `scheduler_error`, `control_unavailable`, `account_changed`, and
  `capacity_retry_exhausted`. Consumers must preserve
  the object but render unknown future state or reason values as a safe generic
  unavailable/failure condition; they must not infer permission to submit from
  an unknown value. `scheduled_at` is present only while the daemon has a next
  scheduled wake, either for a provider reset, an unknown-reset continuation
  probe, or bounded provider-capacity backoff. `failure_reason` is present only when the latest transition records a
  safe operational reason.
- The daemon owns scheduling. It waits for the latest reset among all exhausted
  windows, adds bounded deterministic jitter, re-collects usage at wake, checks
  that the session activity revision is still eligible, and durably claims the
  submission before sending one fixed product-owned continuation message.
  Capacity recovery skips usage collection but retains the same runtime,
  binding, activity revision, pending-attention, manual-input, health, and
  durable pre-submit fences. An indeterminate control submission is terminal
  and is never replayed after restart.
  When an authoritative Claude rate limit has no exhausted percentage window
  or future reset timestamp, the daemon keeps the claim scheduled and uses a
  low-frequency continuation probe (backing off through five, fifteen, thirty,
  and sixty minutes after repeated structured rate-limit failures) until it
  succeeds or a user/session-state change cancels it. On upgrade, the daemon
  recovers the exact pre-upgrade terminal `usage_window_not_exhausted` state only
  while its blocked activity revision is still eligible; other terminal states
  remain fail-closed.
  Restart recovery scans pending records; duplicate events/ticks cannot submit
  twice, cancellation is serialized against wake-up, and bounded retry for
  non-authoritative usage or scheduler failures ends in an observable terminal
  failure.
  Claude usage checks use the existing provider helper. Codex app-server
  sessions use `account/rateLimits/read` on their bound control connection and
  submit the continuation with `turn/start`; only a response carrying the
  acknowledged turn id counts as success. A timeout or disconnect after the
  durable claim is terminal `submission_outcome_unknown` and is never replayed.
- `GET /workdirs?q=...&limit=N` — authenticated read; searches only the default operator roots (`$HOME/Project` and
  `$HOME/.config`) with bounded depth, count, and elapsed-time limits. Add `git_only=true&exclude_worktrees=true` for
  the curated project picker: only primary git working trees are returned, ordered by most-recent session cwd usage
  (`last_used`) and then name/path.
- `GET /codex/accounts` — authenticated nickname-only account inventory from
  the configured host credential broker:
  `{ "machine", "provider": "codex", "accounts": [{ "account", "label"?,
  "plan"? }], "selection_strategies", "readiness" }`. `selection_strategies`
  lists the broker-advertised selectors the daemon understands
  (`current_default`, `default_with_capacity`, `next_with_capacity`); `provider`
  and `selection_strategies` are additive. The `readiness` projection
  reports whether the installed Codex version meets the minimum app-server
  floor and currently advertises Unix listen support, with only a canonical
  provider version and stable safe reason code. A capable CLI also needs a
  usable private runtime directory (see the Codex runtime selection below);
  otherwise readiness reports `supported: false` with
  `codex-app-server-runtime-dir-unavailable`,
  `codex-app-server-runtime-dir-unsafe`, or
  `codex-app-server-socket-path-too-long`. Newer stable Codex releases are
  accepted by capability instead of an exact-version allowlist; exact protocol
  attention remains limited to explicitly audited versions and otherwise falls
  back to hook authority. The response never contains access tokens, ChatGPT
  account ids, auth paths, or broker diagnostics. Without a configured broker
  the route returns `409 codex-account-unsupported`.
- `GET /claude/accounts` — authenticated nickname-only Claude account
  inventory from the configured Claude account broker:
  `{ "machine", "provider": "claude", "accounts": [{ "account", "label"?,
  "plan"? }], "selection_strategies" }` (`provider` is additive). It never
  contains credentials or account directory paths. Without a configured broker
  the route returns `409 claude-account-unsupported`.
- `GET /activity/events` — authenticated metadata-only SSE for activity snapshots and heartbeats. Events carry a daemon-boot
  `stream_id` and increasing `sequence`; `Last-Event-ID` enables count-and-byte-bounded replay, while stale/foreign cursors
  and lagged consumers receive a reset. Concurrent subscribers are daemon-capped and saturation returns a stable
  polling-fallback error. Provider hooks only update durable local activity files; a daemon filesystem watcher publishes changes
  through bounded nonblocking queues. Payloads and SSE frames are serialized once and shared; an oversized snapshot emits a
  transition-only content-free `oversized_snapshot` reset that requires immediate polling rather than entering replay or
  broadcast retention. Notification storms converge through a trailing quiet debounce and refresh starts spaced by an explicit
  minimum cadence. Backend rescan flags force a full refresh; sessions-root loss re-arms a replacement recursive watcher or
  degrades the stream. Watcher or snapshot-source failures send existing streams one reset before closing, stop
  heartbeats, and make new stream requests return the polling-fallback error. The stream and `/sessions` share one snapshot
  source; degraded resets use unique sequences while retaining the last successful snapshot observation anchor, and typed
  projection omits absent nested leaves while preserving session-level `turn_state: null`. The existing `/sessions` read remains
  the old-peer and gap-reconciliation path. The exact
  wire/privacy contract is [activity-stream-v1](activity-stream-v1.md).
- `POST /sessions` (create), `PATCH /sessions/{id}` (title update), `POST /sessions/{id}/send`,
  `POST /sessions/{id}/prompt`,
  `POST /sessions/{id}/resume`,
  `PUT /sessions/{id}/account`,
  `PUT /sessions/{id}/auto-resume`, `DELETE /sessions/{id}/auto-resume`,
  `POST /sessions/{id}/attachments?filename=...`,
  `POST /sessions/{id}/orchestration/group-cleanup`,
  `POST /sessions/{id}/orchestration/group-archive`,
  `DELETE /sessions/{id}` — writes, require a bearer token.
- `GET /sessions/{id}/orchestration/group-cleanup` returns an exact,
  metadata-only cleanup preview for the session's active Main Agent run. The
  plan is fenced by the Main Agent incarnation, run revision, and a SHA-256
  plan digest; it lists only workers whose current primary manager is that
  exact Main Agent. `POST` requires the preview fences, `mode: "safe"|"force"`,
  and an idempotency key. Safe mode rejects nonterminal assignments. Force mode
  records those assignments as cancelled before deleting worker sessions.
  Execution deletes workers first, closes the run only after worker cleanup
  succeeds, and deletes the Main Agent last. A partial result always reports
  `main_deleted: false`; clients must preserve the Main Agent card and surface
  each reported deleted, absent, not-started, or failed worker outcome. Workers
  not yet attempted after a failure remain live and are omitted from that
  partial result.
- `GET /sessions/{id}/orchestration/group-archive` returns the same exact
  worker-first plan under an additive group-archive envelope. `POST` requires
  `agent-session.main-agent-group-archive-request.v1` with the same incarnation,
  run-revision, plan-digest, mode, and idempotency fences. Execution reuses the
  daemon-owned cleanup lifecycle, but prepares each member's provider-history
  archive before that exact runtime is stopped and commits it only after
  deletion succeeds. A missing provider identity or archive write failure
  leaves that member live, returns a retryable partial cleanup result, and never
  falls back to archiving the Main Agent alone. Collaborators, borrowed
  sessions, and workers managed by another Main Agent remain outside the plan.
  Exact retries resume the durable worker-first cleanup receipt; an interruption
  after verified deletion cannot lose the already-written archive metadata.
- `POST /sessions/{id}/prompt` submits exact prompt text through a supported provider control plane. The compatibility route accepts
  `{ "text": "...", "expected_session_incarnation": "launch-id" }`; the incarnation is optional for older clients, and
  a new daemon validates it against the authoritative runtime under the session-record lock before provider dispatch.
  Clients that require a cross-version fence use `POST /sessions/{id}/prompt/v2`, which requires both fields, rejects
  unknown fields, and is absent from older daemons so they fail before provider dispatch. A replacement returns HTTP 409
  `session-incarnation-conflict` without submitting. Success returns `submitted: true` plus the locked
  `session_incarnation`, while the provider turn id remains private. These mutations never send multiline text through
  terminal keys; unsupported or not-yet-ready sessions fail closed.
  On a Claude (pane-delivered) runtime, success means Claude's `UserPromptSubmit` hook started a new turn within the
  acknowledgement wait, or the pane lists the prompt among its queued messages behind a running turn; the queued case adds
  `queued: true` and the prompt runs as the next turn, so the client MUST NOT resend it. A new turn omits `queued`.
  When neither is observed the route answers 500 `structured-prompt-outcome-unknown`.
  For an already resumed Codex runtime whose terminal path reports local
  JSON-RPC `-32001` busy, the error alone does not prove rejection before
  provider acceptance: the same response can be emitted while an earlier
  request is already pending. Current proxies add `error.data.reason` from a
  closed vocabulary without user-controlled or identifying values while
  preserving the stable code and message. The current values are
  `turn_already_pending`, `account_mutation_forbidden`, `account_not_ready`,
  `turn_request_invalid`, `turn_gate_open_failed`, `turn_gate_busy`,
  `manual_marker_thread_mismatch`, `manual_marker_replaced`,
  `manual_marker_invalid`, `manual_ack_failed`, `runtime_identity_missing`,
  `manual_cancellation_busy`, `runtime_changed`, and
  `account_authority_unavailable`. Older proxies may omit `data.reason`, and
  clients MUST tolerate an absent or unknown reason while retaining the stable
  code/message fallback. A durably bound exact-runtime account
  remains valid in a detached tmux scope without inheriting the daemon's broker
  command; the broker is still mandatory for binding and account mutation. A
  client MUST first refresh `GET /sessions` and
  inspect provider-visible activity/output. While a turn is in progress, the
  continuation appears accepted, or non-delivery cannot be established, the
  client waits and MUST NOT submit it again. Only after establishing that no
  provider turn remains in progress and the continuation was not accepted may
  the client bind one explicit recovery request to the exact current
  `session_incarnation` and submit through `prompt/v2`. The incarnation fence
  prevents submission to a replacement runtime; it does not deduplicate two
  submissions within one incarnation. Clients MUST NOT automatically replay an
  arbitrary failed terminal send through this route because transport and busy
  failures can leave provider-delivery outcome unknown. Incarnation conflict
  and outcome-unknown handling retain the rules above.
- `PUT /sessions/{id}/account` accepts
  `{ "account": "nickname", "expected_session_incarnation": "launch-id" }`
  for a serve-managed Codex app-server runtime or a broker-bound Claude
  session; the response carries the provider's `codex_account` or
  `claude_account` projection. Both providers share these outcomes: an invalid
  nickname is `400 invalid-<provider>-account`; a nickname the broker does not
  list is `400 <provider>-account-unknown` (re-selecting the currently bound
  account is always accepted, so it can cancel a queued switch); a stale
  `expected_session_incarnation` is `409
  <provider>-account-session-incarnation-conflict`; a daemon or session
  without that provider's broker is `409 <provider>-account-unsupported`.
  Claude applies a switch by relaunching; see the
  [Claude account broker](#claude-account-broker). For Codex, at the authoritative
  `waiting` boundary, the daemon applies the account immediately without
  recreating tmux or resuming the provider conversation. While a turn is
  `working`, it instead stores an additive durable `next` intent and leaves
  `selected_account` unchanged until that intent applies successfully. This
  also applies to an unbound runtime that inherited the global default: it may
  queue its first explicit account when the request includes
  `supports_unbound_account_queue: true`, without claiming that account is
  already applied. The opt-in keeps daemon-first and edge-first rolling
  upgrades fail-closed. The public `next` projection contains only the desired
  nickname, revision, `queued` / `applying` / `failed` state, and a safe failure
  reason. A successful mutation response also echoes the current
  `session_incarnation`, allowing an intermediary to verify an unbound queued
  outcome against the request fence without exposing credential metadata.
  The control loop drains a queued intent after the live app-server reports no
  in-progress turn for the bound thread, even when the hook-derived activity
  projection has not yet caught up to `waiting`. The TUI proxy serializes
  account mutation with every forwarded `turn/start` until its matching
  provider response, so an accepted-but-not-yet-observed turn cannot cross
  that idle boundary. It resolves credentials through the host broker and
  sends Codex `account/login/start` with `chatgptAuthTokens`. Success flips the durable
  binding and clears `next`; failure preserves the applied binding and marks
  the intent failed. Prompt, terminal-input, and auto-resume submission paths
  fail closed while a selected binding is `pending` or `failed`, or while
  any live or malformed next intent exists, so the next accepted prompt uses
  the newly selected account. An interrupted `applying` intent is re-queued
  when the session runtime restarts.
  The local `agent-session account switch` CLI shares this route's code path.
  Without the daemon's in-process Codex control it always takes the durable
  `next` path above, which the control loop applies before the next prompt.
- `POST /sessions` normally creates a fresh session from `agent`, optional
  `cwd`, `title`, `title_state`, `id`, `prompt`, `coordination_mode`, and
  `agent_args`. `coordination_mode` accepts `advisory`, `enforce`, or `off` and
  defaults to `advisory`. A fresh create may add an advertised `agent_profile`; the id
  must match the supplied base `agent` and be ready when the request arrives.
  A profile whose summary reports `provider_resume_import_supported: true` may
  also be selected with `provider_resume_id`; discovery is then confined to
  that profile's provider root and never falls back to the daemon process's
  base root. The server resolves the profile's executable, provider config root, readiness command, and
  auto-resume capability; callers cannot submit or override those fields. A
  fresh Codex create may additionally provide
  `codex_account`; like `claude_account`, an explicit account must be a valid
  nickname the broker lists (`invalid-codex-account` /
  `codex-account-unknown`, both HTTP 400; `409 codex-account-unsupported`
  without a broker) before anything launches, and when a
  prompt is also present, the daemon completes account binding before
  submitting that prompt. Codex prompt submission also waits for the control
  worker to finish loaded-thread persistence, reconnect/resume, and its initial
  usage wakeup before taking the session-record lock. Control registration alone
  does not establish readiness. Startup failure remains
  `409 structured-prompt-unavailable`; the final locked incarnation and account
  checks still fence submission. A fresh, profile-free Claude create may provide
  `claude_account`; see the [Claude account broker](#claude-account-broker).
  Each account field is rejected for another provider
  (`<provider>-account-agent-conflict`) and in provider-import mode
  (`<provider>-account-provider-resume-conflict`). Only `claude_account` is
  also rejected with an `agent_profile` (`claude-account-profile-conflict`),
  intentionally: a Claude launch profile owns the provider config root that an
  account binding would replace, while Codex credentials are injected through
  the app-server control plane and compose with a profile.
  When `provider_resume_id` is present (alias: `resume_id`), the daemon imports an existing Codex or
  Claude provider conversation instead: it resolves the original cwd from the selected local provider history, persists exact
  `provider_resume` metadata, and starts tmux with the canonical resume command. A capable Codex import uses the daemon-managed
  app-server transport so account and auto-resume controls remain available; unsupported or explicitly raw Codex runtimes retain
  the standalone resume fallback. In resume-id mode, omit `cwd`, `prompt`,
  and `agent_args`; invalid, missing, ambiguous, or unsupported provider ids return structured errors.
  Either mode may add `lineage` (who started the session) and `work` (its
  program and issue references), validated and stored as
  [Session lineage and work v1](session-lineage-work-v1.md#serve-create)
  defines (`lineage-invalid`, `lineage-depth-exceeded`, `work-ref-invalid`,
  all HTTP 400). Without `lineage` the session is an operator root over HTTP.
  The response's `session` echoes both.
  For a serve-managed Codex session, including provider imports, `agent-session` probes bounded
  `codex --version` and `codex app-server --help` process groups. App-server
  transport requires Codex `>= 0.144.1` and advertised Unix `--listen` support.
  Exact protocol-attention authority is audited only for Codex `0.144.1` and
  `0.144.3`; newer transport-compatible versions fall back to hook authority.
  An eligible CLI is launched as a remote TUI over a private short socket below an
  owned, non-symlinked mode-`0700` runtime root: an absolute `XDG_RUNTIME_DIR`
  when set, otherwise a platform default created mode `0700` (the daemon state
  directory's `run/`, or `/tmp/agent-session-<uid>` when the state directory is
  too long for a Unix socket). The default passes the same validation;
  otherwise auto mode degrades to the existing raw TUI and records the reason
  in `startup.runtime_fallback`. `AGENT_SESSION_CODEX_RUNTIME=raw` forces
  the fallback and `AGENT_SESSION_CODEX_RUNTIME=app-server` requires both the
  same capability probe and a private Unix socket. Standalone `agent-session
  start` remains raw because no serve daemon owns its control connection.
  The remote TUI connects through a private mode-`0600` WebSocket bridge to the
  private app-server socket. The bridge forwards frames unchanged, observes the
  exact TUI connection's structured lifecycle metadata through bounded
  background projection, and discards message content after in-memory
  reduction. Projection loss disables an existing claim without interrupting
  the TUI transport. Direct TUI thread/turn creation is launch-fenced against
  auto-resume before forwarding. A direct turn waits up to one second for
  transient state-lock contention before returning Busy; create-bootstrap and
  sender-owned manual-input gates remain non-blocking, and persistent
  contention still rejects only that request. This keeps an immediate
  post-interrupt turn from receiving a fatal transient RPC error without
  bypassing lifecycle authorization. Control-plane Enter injection (either a
  named Enter key or the raw CR/LF frame emitted by an attached terminal)
  already performs the same cancellation while holding the lifecycle lock. A
  live proxy advertises
  this coordination capability with a private, launch-bound, file-locked
  marker; older live proxies reject HTTP submission, while attached input
  disconnects without mutating auto-resume and requires session recreation.
  Immediately before the submitting tmux operation, the sender opens a bounded
  manual-input section. If ordinary
  proxy cancellation reports Busy, only a valid `turn/start` for the exact
  bound thread may hold that section's gate while it is forwarded. Gate teardown
  completes before the sender releases the lifecycle lock and removes the
  marker, preventing stale authorization of another lock holder. These markers
  store no prompt, terminal content, or raw thread id; malformed, expired,
  dead-process, and replacement-runtime state fails closed. The visible TUI
  creates the fresh thread;
  neither the bridge nor the control client synthesizes a shell or model turn.
  A separate daemon control connection reads usage and submits a continuation
  on that bound thread. Only a mode-`0600` SHA-256 thread binding is persisted,
  so reconnects fail closed on a mismatch and raw thread ids are never stored.
  The bridge remains with the tmux runtime across daemon restarts. Runtime paths
  are namespaced by state and launch identity; delete and launch failure
  validate and remove the app-server socket, bridge socket, and marker paths.
  A selected account is persisted only as nickname, revision, public state
  (`unsupported`, `unbound`, `pending`, `bound`, or `failed`), and applied
  launch id. New default-account sessions resolve and bind the current global
  default before their first prompt, then project the nickname as both
  `selected_account` and `effective_account` with
  `selection_source: "default_at_launch"`. Explicit and automatic selections
  use `explicit` and `auto_failover`; historical unbound records remain honestly
  unknown rather than being inferred from the current global default. On daemon reconnect or stopped-session resume, the new control
  connection re-applies that nickname before accepting input. Codex
  `account/chatgptAuthTokens/refresh` with reason `unauthorized` triggers one
  forced broker refresh and the same durable pending/bound transition. If that
  refresh fails before new credentials reach the exact runtime, the daemon
  restores the prior bound identity at the newer revision so a later provider
  request can retry; a superseding account or runtime change wins instead.
  Initial binding and explicit account-switch failures remain fail closed.
- Session reads include a monotonic `title_revision`. `PATCH /sessions/{id}` may include
  `expected_title_revision`; a stale value returns `409 title-revision-conflict` without changing the title.
  Upgraded clients also send the runtime's random `session_incarnation` as `expected_session_incarnation` and the
  observed title as `expected_session_title`. A different runtime UUID rejects delayed requests aimed at a
  deleted-and-recreated or resumed session with `409 session-incarnation-conflict`; exact title comparison rejects
  changes made by older daemons that do not advance the revision with `409 title-state-conflict`.
  `expected_session_created_at` remains accepted for transitional clients. Omitting these fields preserves
  unconditional updates for backward-compatible clients.
- Session create and PATCH requests may provide `title_state` instead of deriving semantics from the rendered title.
  Its shape is `{ "topic": string|null, "topic_source": "none"|"auto"|"user", "references": ["#123"],
  "activity": string|null }`. A `user` topic is client-owned and stable; an `auto` topic may be revised as the session
  converges; `none` requires a null topic. The daemon validates at most two numeric work-item references and renders the
  compatibility `title` as `<topic and references> - <activity>`, or as the only non-empty side when one side is absent.
  Supplying both fields requires an exact canonical match. Title-only compatibility writes remain accepted and clear
  `title_state`, so old clients never leave structured provenance attached to an unrelated title. Reads omit stale
  structured state if an older writer changed only the compatibility title.
  Session and glance responses advertise `title_state_supported: true` independently of whether that session already
  has structured state, allowing upgraded clients to migrate title-only records conservatively.
- `POST /sessions/{id}/send` accepts at most 64 entries in `keys`; a longer list is
  rejected with `400 too-many-keys` and a `limit` detail before the daemon sizes any
  allocation from the request. Every accepted name resolves to one of the
  canonical `SpecialKey` values, so the bound is far above any real caller. An
  unknown name still fails the whole request with `400 invalid-key`.
- `POST /sessions/{id}/send` pastes `text` with bracketed-paste markers. A request
  with literal `text` and `keys` exactly `["enter"]` is a prompt submission and
  `data.sent.submission` reports `{ outcome, enter_presses }`. `outcome` is
  `submitted`, `queued` (Claude Code queued it behind a running turn), or
  `unverified` (the pane could not prove either way). A prompt still in the
  composer after the bounded Enter retries returns `409 send-submit-stuck` with
  `{ id, outcome: "stuck", enter_presses }`. The text has reached the pane, so
  do not resend it. Inspect or clear the composer first. Retries never press
  Enter while the session is `needs_input` or the pane shows a dialog.
- Blocked-input contract. While a session's turn phase is `needs_input` — reached
  only through a provider `attention_requested` event, never a terminal
  heuristic — the pane belongs to an approval or question dialog rather than to a
  prompt box, so text delivered into it answers that dialog instead. The refusal
  is therefore scoped to routes that **write to the pane**, and reports
  `409 agent-blocked` with a `{ id, phase, remedy }` detail before anything
  reaches the terminal:

  | Route | While `needs_input` |
  | --- | --- |
  | `POST /sessions/{id}/send` carrying literal `text` | refused, unless `allow_blocked: true` (CLI `--allow-blocked`) |
  | `POST /sessions/{id}/send` with keys only | admitted — answering the dialog is what a blocked session stays addressable for, including the `enter` that confirms a highlighted choice |
  | `POST /sessions/{id}/send` whose `text` is only a newline | admitted; it is delivered as that Enter keypress rather than typed characters |
  | `POST /sessions/{id}/prompt`, `/prompt/v2` on a pane-delivered (Claude) runtime | refused, no opt-in; use `send` with `allow_blocked` to type into the dialog |
  | `POST /sessions/{id}/prompt`, `/prompt/v2` on a Codex app-server runtime | admitted — it submits over the control channel and never touches the pane |
  | `PATCH /sessions/{id}` title rename projection | suppressed; the title still persists and the pane-side name updates after the dialog is answered |
  | `GET /sessions/{id}/attach` input frames | admitted — see the exemption below |

  Every phase other than `needs_input` is admitted, as is an absent or degraded
  turn state: blockedness is unknowable without valid provider evidence, and
  failing closed there would strand providers with no turn tracking.

  The pane-delivered prompt is checked twice — once as an early reject, then
  authoritatively inside the record lock that also fences the write — so an
  `attention_requested` ingest arriving mid-request cannot slip a paste through.

  Coordination notifications never reach this refusal, but not because they
  always require a `waiting` recipient: the dispatcher routes a supported Codex
  runtime whose phase is `working` or `needs_input` with authoritative confidence
  to its fenced in-turn checkpoint, and only otherwise requires `waiting`.
  Neither path goes through `/send` or `/prompt`.

  **This is a safety default, not an authorization boundary.** The attach socket
  is an interactive terminal and accepts text frames under the same bearer token,
  so any caller that `agent-blocked` refuses can perform the same pane write over
  `/attach`. The refusal stops an automated caller from typing into a dialog by
  accident; it does not stop one that means to. A consumer whose HTTP `/send`
  call carries human keystrokes — an interactive terminal falling back from a
  closed WebSocket, for example — should send `allow_blocked: true`, because that
  input is deliberate by construction.
- Attachment upload uses a raw binary request body (not multipart). The daemon
  streams it into a private same-directory temporary file, enforces the declared
  and observed byte ceiling, syncs it, and publishes it without replacing an
  existing attachment. The default ceiling is 1 GiB;
  `AGENT_SESSION_MAX_ATTACHMENT_BYTES` may set an integer from 1 byte through
  16 GiB before daemon startup. Oversize input returns
  `413 attachment-too-large`; a request-body failure returns
  `400 attachment-read-failed`; neither leaves a partial file. The returned
  serve envelope contains the sanitized filename, exact byte count, and remote
  path under the session's private `attachments/` directory.
  Empty or null titles clear the custom session title so clients can fall back to the session id.
- `GET /sessions/{id}/attach` — a WebSocket PTY attach: a `capture-pane` snapshot then a live byte stream from one
  daemon-owned `tmux pipe-pane` broker per session (binary frames, renderable by xterm.js). Concurrent clients fan out
  from the same bounded in-memory stream; disconnecting one client leaves the others live, while a lagging client is
  disconnected so it cannot stall tmux or other clients. The broker uses a private ephemeral FIFO and retains no
  interactive-session terminal bytes after the final client disconnects. Snapshot capture drains live output into a
  bounded handoff buffer and performs one bounded fresh-snapshot recovery if that buffer overflows. After handoff, a
  supervised per-client pump keeps draining broker output independently of provider discovery, input, and resize work;
  a normal broker close drains already accepted frames under the WebSocket send bound, while lag/error teardown remains
  immediate. The client sends JSON control frames
  `{ "text": "...", "key": "enter", "keys": ["c-c"], "resize": { "cols": 80, "rows": 24 } }`. Token-gated; disconnect
  leaves the tmux session alive. The named key `shift-left` uses tmux `S-Left`,
  honoring the pane's negotiated keyboard mode. For older clients, an exact
  `text` frame containing `ESC[1;2D` also means Shift+Left; the sequence inside
  other text or bracketed-paste markers remains literal. Both forms use the same
  serialized session input and runtime fencing as other keys.
  Concurrent clients share the pane geometry; resize sequences are serialized and the
  last completed resize wins. A client may opt into authoritative Codex/Claude prompt events by sending
  `{ "subscribe": ["provider-prompt.v1"] }`. For a known, resumed, imported, or reconnected provider session, the daemon
  baselines the exact provider transcript at EOF. When a generation-1 fresh Codex/Claude runtime is still establishing its
  exact provider identity or transcript, the connection instead keeps a bounded, cancellable resolver alive, reloads the
  same launch's session metadata, and opens that fresh transcript from its beginning so the first prompt is not lost.
  Codex `UserPromptSubmit` hook metadata supplies the exact runtime-bound session identity; the pending attach path never
  promotes cwd/time history scans into beginning-of-transcript authority. Transcript discovery is shared by runtime across
  attach clients, is owned by the daemon across waiter cancellation, uses bounded exponential backoff, and admits at most
  four concurrent history scans. Active slots are never capacity-evicted; obsolete runtime keys and deleted sessions are
  evicted, and a fixed daemon-local entry cap bounds unrelated session churn. Reconnects
  baseline the revalidated cached exact source at EOF. Passive list/glance reads never persist heuristic Codex history
  into a live generation-1 runtime, and fresh Codex byte-zero recovery admits only `codex-user-prompt-submit-hook`
  identity. Explicit stopped-session resume may recover older provider history while holding the same per-session record lock for
  its complete resume/rollback transition. It
  replies with an `agent-session.attach.v1` `capability` text frame once resolution finishes, and only after that
  acknowledgement emits `prompt_submitted` events as bounded text frames; terminal snapshot/live output remains binary.
  The normative supported acknowledgement is:

  ```json
  {
    "schema_version": "agent-session.attach.v1",
    "type": "capability",
    "capability": "provider-prompt.v1",
    "supported": true,
    "provider": "codex",
    "prompt_max_bytes": 16384
  }
  ```

  `provider` is `"codex"` or `"claude"` when supported and `null` otherwise; an unsupported provider, unresolved exact
  transcript, unsafe transcript path, exhausted discovery budget, or expired fresh-runtime resolution returns the same
  object with `supported:false`. The fresh-runtime resolver is restricted to the original generation-1 launch identity:
  Codex must have had no provider identity when the client subscribed, and Claude must carry the daemon-generated
  `claude-explicit-session-id` capture method. Imported, resumed, replaced, and later-generation runtimes never enter the
  beginning-of-transcript path, so reconnect retains EOF/no-history behavior. While resolution is pending, terminal bytes,
  input, resize, and broker fanout continue independently; consumers may activate their bounded local fallback before the
  eventual acknowledgement.
  Clients that do not subscribe receive no event text frames. The normative event is:

  ```json
  {
    "schema_version": "agent-session.attach.event.v1",
    "type": "prompt_submitted",
    "event_id": "pp-opaque",
    "provider": "codex",
    "submitted_at": "2026-07-10T03:51:49Z",
    "text": "final provider-recorded prompt",
    "truncated": false
  }
  ```

  `event_id` is unique and opaque, `submitted_at` uses the provider timestamp when present (otherwise detection time), and
  `text` is UTF-8 bounded to `prompt_max_bytes`; `truncated` reports clipping. Events never contain transcript paths and are
  never logged or persisted by the daemon. Terminal and control queues are bounded: terminal frames receive bounded burst
  preference, while advisory prompt events may be dropped on saturation and must not delay terminal bytes. Consumers should
  retain their documented local fallback when capability is absent/false or an event does not arrive within its bounded
  fallback interval.

### Provider usage and resets

`GET /usage/v1`, `POST /codex/reset/v1`, and `POST /claude/reset/v1` serve
provider usage, the earned Codex rate-limit reset, and the Claude limit
resets from the `codex-cli` and `claude-cli` provider CLIs on `PATH`, so a console edge needs no separate host helper. All three require the
server bearer, like every other authenticated route. Their response shapes
match what a console edge already parses from the helpers they replace; the
synthetic fixtures under `tests/fixtures/usage-v1/` pin that projection.

**Usage.** `GET /usage/v1` returns the ordinary serve envelope with
`data.machine` and `data.usage`:

```json
{
  "schema_version": "agent-session.provider-usage.v1",
  "providers": [
    {
      "provider": "codex",
      "account": "alpha",
      "label": "Codex",
      "ok": true,
      "stale": false,
      "plan": "team",
      "windows": [
        { "key": "5h", "label": "5h", "used_percent": 6, "window_minutes": 300, "resets_at": 1790003600 }
      ],
      "updated_at": 1790000000,
      "note": null,
      "error": null,
      "reason_code": null,
      "reset_credits": { "available_count": 2 }
    }
  ]
}
```

- Codex contributes one entry per account from
  `codex-cli diag rate-limits --all --format json --no-refresh-auth`, in the
  helper's order. `account` is the profile nickname, `plan` only a known
  ChatGPT plan tier, and `reset_credits` only a non-negative integer count;
  absence means unknown, not zero. Window labels must be `Weekly` or a
  provider duration such as `5h`, from which `key` and `window_minutes` are
  derived. An account the helper reports as failed is `ok: false` with a
  fixed `error` text and its `reason_code` (`unknown` when unclassified).
  Account results are preserved even if the aggregate helper `ok` is false or
  its exit status is nonzero; each failure affects only that account. A valid
  all-failed account list also stays per-account. That run also refreshes the
  shared rate-limit cache that `codex-cli account select` reads, so an account
  selection within the cache TTL does no provider fetch.
- Claude contributes one entry per valid profile nickname from
  `claude-cli diag rate-limits --async --format json --jobs 16`. The bounded
  parallel run disables cache fallback on profile failures so an old healthy
  cache cannot mask the failed account's reason. A successful network result
  is a fresh `ok: true` entry with that nickname in `account`. The active
  Claude Code login is also included when its access token is not already
  represented by a saved profile. It uses `account: "active"` unless that
  nickname is already used by a profile, in which case it uses an unused
  `active-login` nickname; an optional
  helper-supplied plan is retained only when it is a bounded token. A
  profile-level failure is an `ok: false`, `stale: true` entry
  with the account nickname, a fixed `error` and `note`, and its classified
  `reason_code`; an unclassified profile failure uses `unknown`. A successful
  result without windows remains `ok: true` and stale with a fixed `note`.
  A successful no-window result may use the helper's cached fallback; these
  cached results are stale and their windows are hidden because the helper
  does not expose the cache timestamp.
- A helper can fail to run, time out after 30 seconds, or return an unusable
  document. If the provider has no earlier success, or its last success is
  600 seconds old or more, it is then reported as one entry with
  `account: null`, `stale: true`, and a `reason_code` of `service_unavailable`,
  `timeout`, or the helper's own classified reason. That entry replaces any
  per-account entries because the helper did not provide a usable account
  inventory. Both providers mark it `ok: false` and include a fixed `error`.
- `reason_code` is always `null` or one of `auth_required`, `auth_expired`,
  `billing_past_due`, `subscription_inactive`, `organization_disabled`,
  `permission_denied`, `rate_limited`, `service_unavailable`, `timeout`, or
  `unknown`. `note` and `error` are fixed daemon-owned strings; helper
  messages never pass through. Output never carries credentials, provider
  account ids, emails, file paths, or raw provider responses.

**Caching.** Each provider has a stale-while-refresh cache. A completed
snapshot is fresh for 60 seconds (`AGENT_SESSION_USAGE_V1_REFRESH_SECONDS`,
1 to 300). The first read after that starts one background refresh and
immediately gets the last completed snapshot, with every `ok` entry marked
`stale: true` and a refreshing note. At most one refresh per provider runs at
a time. A failed refresh keeps serving that snapshot with a backoff note and
retries after one more interval. Windows are hidden (and the entry marked
stale) once their `updated_at` is 600 seconds old or more than 5 seconds in
the future, so a persistent outage never pins old numbers. `?refresh=1`
marks the current snapshot stale and waits up to 7 seconds for a refresh that
starts after the request. If a refresh is already running, one more run
starts when it finishes, so a forced read never returns numbers from before
the request. A cold daemon waits the same bound for its first result. Any
other query is `400 invalid-query`.

**Codex reset.** `POST /codex/reset/v1` takes exactly
`{"account": "<nickname>", "idempotency_key": "<uuid>"}` and consumes at most
one earned reset through
`codex-cli account reset-rate-limits --yes --idempotency-key <uuid> --format json <nickname>.json`.

- `AGENT_SESSION_CODEX_RESET_ACCOUNTS` is the allowlist of nicknames,
  separated by spaces or commas. Unset or empty disables the route with
  `503 codex-reset-not-configured`; an unlisted account is
  `403 codex-reset-account-not-allowed`.
- The idempotency key must be a canonical lowercase UUID. The provider
  receives it as the redemption id, so a repeated key never consumes a second
  credit. The daemon serializes resets and replays a recorded outcome for the
  same key and account for 24 hours (`replayed: true`, no second CLI run); the
  same key for another account is `409 idempotency-key-reused`. A reset keeps
  running and is recorded even when its caller disconnects, so a retry with
  the same key replays it. Recording a reset also marks the Codex snapshot
  stale and starts a refresh, so neither a replay nor a later read serves
  pre-reset numbers as fresh. A failed run is not recorded, so the caller retries
  with the same key.
- A malformed body is `422 invalid-request`. CLI failures are
  `502 codex-reset-failed`, `502 codex-reset-invalid-response`,
  `502 codex-reset-unavailable`, or `504 codex-reset-timeout` (30 seconds, an
  unknown result to retry with the same key). Errors use the ordinary serve
  error envelope and carry no helper output.

Success is not wrapped in the serve envelope. Like the federated mailbox
routes, it returns a raw versioned document whose `schema_version` is the one
the console reset client checks:

```json
{
  "schema_version": "agent-console.codex-rate-limit-reset.v1",
  "outcome": "reset",
  "windows_reset": 2,
  "replayed": false,
  "machine": "workstation",
  "usage": { "schema_version": "agent-session.provider-usage.v1", "providers": [] }
}
```

`outcome` is `reset`, `nothing_to_reset`, `no_credit`, or `already_redeemed`,
and `windows_reset` is present only when the CLI reports it. `usage` is the
`GET /usage/v1` snapshot after a forced Codex refresh. The response waits for
that refresh only until 6 seconds after the request arrived, so a client with
an 8-second timeout still receives the outcome. A slower refresh leaves the
Codex entries `stale: true` with the refreshing note. A replay returns the
cached snapshot instead.

**Claude reset.** `POST /claude/reset/v1` takes exactly
`{"account": "<nickname>", "program": "juniper_tide" | "cedar_ember", "idempotency_key": "<uuid>"}`
and redeems at most one Claude limit reset through
`claude-cli auth reset-rate-limits --yes --program <program> --request-id <uuid> --format json <nickname>`.
`juniper_tide` is the weekly reset of the 5-hour session limit and
`cedar_ember` the next granted reset; `claude-cli` picks the grant on the
host, so grant ids never cross this boundary.

- `AGENT_SESSION_CLAUDE_RESET_ACCOUNTS` is the allowlist of stored
  `claude-cli` profile nicknames, separated by spaces or commas. Unset or
  empty disables the route with `503 claude-reset-not-configured`; an unlisted
  account is `403 claude-reset-account-not-allowed`.
- Replay works as for Codex, in a separate 24-hour record: the same key for
  the same account and program replays the recorded outcome
  (`replayed: true`, no second CLI run), and the same key for another account
  or program is `409 idempotency-key-reused`. The key is also the
  `cedar_ember` request id the provider receives. A reset keeps running and
  is recorded when its caller disconnects; a failed run is not recorded.
  Recording a reset marks the Claude usage snapshot stale and starts a
  refresh, without waiting for it.
- A malformed body is `422 invalid-request`. CLI failures are
  `502 claude-reset-failed`, `502 claude-reset-invalid-response`,
  `502 claude-reset-unavailable`, or `504 claude-reset-timeout` (35 seconds,
  an unknown result to retry with the same key). serve runs the CLI with
  `CLAUDE_PROMPT_SEGMENT_MAX_TIME_SECONDS=5` and
  `CLAUDE_RATE_LIMITS_RESET_MAX_TIME_SECONDS=25`, overriding its own
  environment, so the CLI finishes its status read and POST before that
  deadline. `claude-reset-failed` adds
  `error.details` when the CLI reported one of its documented error codes:
  `{"cli_code": "<code>", "reason_code": "<reason>" | null, "retryable": <bool>}`.
  `cli_code` is one of `claude-auth-required`, `provider-unavailable`,
  `provider-rejected`, `invalid-provider-response`, `profile-not-found`,
  `profile-invalid`, `organization-unknown`, `endpoint-invalid`,
  `invalid-profile-name`, `confirmation-required`, `request-id-required`, or
  `invalid-request-id`; `reason_code` uses the usage reason vocabulary; and
  `retryable` is true only for `provider-unavailable`. Helper messages never
  pass through.

Success is a raw versioned document:

```json
{
  "schema_version": "agent-console.claude-limit-reset.v1",
  "program": "cedar_ember",
  "outcome": "reset",
  "posted": true,
  "reason": null,
  "resets_left": 1,
  "next_available_at": null,
  "cooldown_until": null,
  "weekly_resets_at": 1791334800,
  "replayed": false,
  "machine": "workstation"
}
```

`outcome` is `reset`, `already_used`, `not_limited`, `cooldown`,
`ineligible`, or `unavailable`. `posted` is `false` when `claude-cli` found
the program unavailable and sent nothing. `reason` is `null` or a bounded
lowercase token, `resets_left` a non-negative integer or `null`, and the
three timestamps epoch seconds or `null`. Every field is always present. The
daemon accepts only a CLI result whose envelope, program, outcome, and field
types match; anything else is `claude-reset-invalid-response`.

## Response and authentication

Ordinary JSON HTTP responses use the `cli.agent-session.serve.v1` envelope.
Successful responses carry a `machine` identity in `data.machine` (`--machine`
/ `AGENT_SESSION_MACHINE` / `--host` / hostname) so an edge can aggregate
several machines; current error envelopes omit the machine field. The activity
SSE stream uses [activity-stream-v1](activity-stream-v1.md), while WebSocket
attach uses the binary/control and optional event frames documented above;
neither streaming transport uses the ordinary JSON envelope. Auth is a bearer token
(`--token-stdin`, `--token`, or `AGENT_SESSION_TOKEN`) on the activity stream plus all write and attach endpoints, compared without an early-exit
on the token bytes; when no token is configured (or it is empty) those endpoints fail closed (503). Prefer
`--token-stdin` for launcher integrations so token material does not appear in process arguments. It reads one trimmed
token from stdin, rejects empty input, rejects multiple newline-separated tokens, and rejects input over 8192 bytes.
`--token-stdin` conflicts with `--token`; both forms avoid printing token material in errors. Use a strong,
high-entropy token.

`--token` and `AGENT_SESSION_TOKEN` remain accepted compatibility inputs.
`--token` exposes the bearer to same-host process-argument inspection. The
environment form can be inherited by managed tmux/provider children because the
daemon does not currently scrub it before launch, which would grant those
children machine-operator authority. Deployments that create sessions must use
`--token-stdin` from a private, non-exported credential source.

## Account brokers

A provider's account broker is enabled by one environment variable, or by the
matching `serve --config` table: `AGENT_SESSION_CODEX_ACCOUNT_BROKER` /
`[codex_account_broker]` and `AGENT_SESSION_CLAUDE_ACCOUNT_BROKER` /
`[claude_account_broker]`. The value is a JSON argv array, never a shell
command, for example `["/opt/agent-console/bin/codex-account-broker"]`, of at
most 16 non-empty arguments of at most 4096 bytes each. The daemon appends a
verb, its arguments, and `--format json`. Every call runs in a fresh process
group with a null stdin, at most 1 MiB of stdout and stderr, and a 10 second
deadline (8 seconds for a Codex forced refresh); a timed-out broker's whole
process group is killed. Broker stderr is never projected.

Both brokers fail closed. Invalid configuration is
`<provider>-account-broker-invalid-config`; a broker that cannot start is
`-unavailable`; a non-zero exit is `-rejected`; a timeout is `-timeout`; and
malformed or oversized output, a wrong `schema_version` (or, for v2, a wrong
`provider`), and a listed account with an unsafe or duplicate nickname or
oversized or multi-line public metadata are all
`<provider>-account-broker-invalid-response`, a broker fault rather than a
client error. Account nicknames follow one rule for both providers and for the
provider CLIs: `[A-Za-z0-9][A-Za-z0-9._-]{0,63}`, so a nickname can never read
as an option (`--format`) or a dot path segment. Credential values and account
directory paths stay in daemon memory or durable session state only and are
never added to HTTP projections.

A missing broker is one condition with one status, `409
<provider>-account-unsupported`, on every route that needs it: `GET
/{provider}/accounts`, `PUT /sessions/{id}/account`, `POST /sessions` with an
explicit `<provider>_account`, and `POST /sessions/{id}/resume` of a session
bound to that provider's account. The same applies to `409
<provider>-account-session-incarnation-conflict`.

serve records the effective broker argv for each provider, from its
environment or `--config`, in the owner-private
`<state-dir>/serve/account-brokers.json` (`agent-session.serve-account-brokers.v1`,
mode `0600`) at startup, and removes it when no broker is configured. Owner-run
`agent-session account` and `agent-session resume` commands do not inherit
serve's environment; they adopt that record for any provider whose broker
variable is unset in their own environment, and ignore a record that is not a
private, owner-owned regular file.

The per-provider differences are the protocol and what the broker returns.

### Codex account broker

The Codex broker speaks `agent-session.codex-auth-broker.v1`, without a
`--provider` argument. The daemon invokes `list`, `resolve`, or a bounded
`select` request. `select --strategy current_default` returns the configured
nickname matched by the live default; `select --strategy next_with_capacity
--after <nickname> [--exclude <nickname>]...` walks configured order once and
returns only a fresh, network-confirmed usable account. `list` returns public
`accounts`, while `resolve` returns the exact nickname plus `access_token`,
`chatgpt_account_id`, and optional `plan`; tokens are resolved on demand and
kept in memory. The additive list field `selection_strategies` advertises these
selectors. Choosing the automatic default for a new session is intentionally
best effort: a daemon paired with an older broker that omits the field, or a
broker that cannot list, leaves the new session unbound on the host login
instead of failing the create. An explicit account, an account switch, and a
selector failure after capability is advertised remain fail closed.

### Claude account broker

The Claude broker speaks the provider-neutral
`agent-session.account-broker.v2` contract. Every call passes
`--provider claude` after the verb:

- `list --provider claude --format json` returns
  `accounts: [{account, label?, plan?}]` and `selection_strategies`
  (`current_default` is the only strategy the daemon uses).
- `select --provider claude --strategy current_default --format json`
  returns `account`.
- `materialize --provider claude --account <nickname> --format json` returns
  `account`, which must echo the request, and `config_dir`.

Every response also carries `schema_version` and `"provider": "claude"`.

No token ever crosses this broker. `materialize` prepares a per-account Claude
configuration directory and returns only its path. Before any Claude process
runs in it, the daemon requires `config_dir` to be absolute and normalized, a
real directory (not a symlink) owned by the daemon user and not world-writable,
holding a regular, non-symlink `.credentials.json` owned by the same user.
Otherwise it fails with `claude-account-dir-unsafe` and a safe `reason`. A
mismatched nickname is `claude-account-broker-invalid-response`.

- Create: an explicit `claude_account` wins; otherwise, when `list` advertises
  `current_default`, the daemon records that account as `default_at_launch`.
  Without a configured broker, a Claude session keeps the host login exactly as
  before, and an explicit `claude_account` fails with
  `409 claude-account-unsupported`.
- Launch: the account is materialized before tmux starts and the provider runs
  with `CLAUDE_CONFIG_DIR=<config_dir>`. The session record keeps a durable
  `agent-session.claude-account-binding.v1` binding (nickname, selection
  source, revision, the runtime it was applied to, and the directory).
- Resume: every resume re-materializes the bound nickname. A bound session
  whose broker is no longer configured fails closed with
  `409 claude-account-unsupported` rather than falling back to the host login.
- Projection: Claude sessions carry an additive `claude_account` object
  (`agent-session.claude-account.v1`: `supported`, `state`
  `bound`/`unbound`/`failed`/`unsupported`, `selected_account`,
  `selection_source`, `revision`, `applied_runtime_id`, optional
  `next: {account, revision, state: "queued" | "failed", failure_reason?}`). It is omitted for other
  providers and for Claude sessions without binding state on a daemon without
  a Claude broker. It never contains the directory path.
- Switch: `PUT /sessions/{id}/account` with
  `{ "account", "expected_session_incarnation" }` on a Claude session durably
  queues `agent-session.claude-account-next.v1`; requesting the bound account
  cancels a queued intent. A nickname the broker does not list is refused
  with `claude-account-unknown`. Claude Code has no live credential swap, so the switch is applied only by a
  relaunch: when the session is running and its turn is `waiting`, the daemon
  first materializes and validates the new account directory. A refusal
  returns `claude-account-switch-refused` (HTTP 422, with the broker's
  `cause` code) before anything is stopped or written, so the running session,
  its binding, and any previously queued intent stay unchanged. Otherwise it
  stops the runtime through the verified stop path and resumes the same
  conversation (`--resume <session-id>`) in the new account directory, then
  returns the new `session_incarnation`. While a turn
  is busy, or while the session is stopped, the intent stays queued and the
  next resume applies it. If the verified stop is refused, nothing relaunches
  and the intent stays queued. After a verified stop, the switch retires the
  stopped runtime's coordination incarnation (its heartbeat writer died with
  it) before resuming, so the resume is not refused as a live prior
  incarnation. If the resume still fails and the current runtime is proven
  stopped, the response is
  `claude-account-switch-resume-failed` (HTTP 422) with
  `details: {id, session_state: "stopped", next_account, cause, recovery}`:
  the next account is retained with `state: "failed"` and `failure_reason`
  carrying the underlying error code. The account projection's state is
  `failed`, and the session's `startup` projection reports
  `claude-account-switch-resume-failed`, so later Console reads expose the
  failure. `recovery` names the
  `agent-session resume <id>` command that applies it. If replacement cleanup
  remains live or unverified, the response preserves the original termination
  or rollback error instead of claiming a stopped session. The account attempt
  is still marked failed with that code; startup reports
  `claude-account-switch-cleanup-incomplete` with `retry_safe: false`; the durable
  startup record retains bounded cleanup state. The local
  `agent-session account switch` CLI shares this path; run from inside the
  session's own tmux session it only queues, since restarting the runtime that
  hosts it would stop the switch mid-way. There is no automatic failover for
  Claude.
- History: catalog scans attribute each physical transcript once, by its
  canonical path, so an account directory whose `projects/` links to the
  shared `~/.claude/projects` is not counted twice.

## Launch profiles

Server-owned launch profiles are configured with
`AGENT_SESSION_LAUNCH_PROFILES`, a JSON array. For example:

```json
[{"id":"custom-claude","label":"Custom Claude","agent":"claude","agent_bin":"/opt/agent/bin/custom-claude","provider_config_dir":"/srv/agent/claude","readiness_args":["--check"],"auto_resume_supported":false}]
```

The daemon rejects malformed, duplicate, relative-path, or over-bounded
configuration at startup. A profile is advertised only when its executable is
an executable regular file, its optional provider config root is a directory,
and the optional readiness argv exits successfully within two seconds. The
readiness argv always runs against `agent_bin`; no shell is involved. Profile
discovery probes are single-flight; concurrent session-list reads share the
same in-flight result, while a later read probes fresh. Profile paths and readiness
details never enter HTTP responses. The safe id and any configured private
provider root persist with the runtime and its durable resume sidecar, so exact
binary and transcript discovery survive daemon restarts. Both daemon-managed
and standalone `agent-session resume <managed-id>` launches pin the persisted
provider root, when present, in the provider-specific environment before
invoking the durable launcher. The daemon resume endpoint additionally requires
the same id, base agent, executable, optional config root, and auto-resume
capability to remain present in the current server registry. It runs the
readiness argv from the current profile and requires that probe to succeed, but
does not persist or compare the launch-time readiness argv. Removing the profile
or changing a persisted identity field revokes resume through that endpoint;
changing only the readiness argv is accepted when the new probe succeeds.
Because standalone resume cannot enforce the live registry, the daemon does not
advertise it as a managed copy action. Set
`auto_resume_supported` only when the profile has authoritative usage semantics
for its provider; the default is fail-closed `false`. A profile with this
capability disabled can still create its initial managed Codex thread and turn;
it does not gain automatic continuation support.

The retired base agent `hermes` is refused: serve does not start with a
profile that names it.

A profile with base agent `dsh` may add
`"dsh_history":{"command":"/absolute/dsh-runtime-kit-history","root":"/absolute/dsh-sessions","compression":"zstd"}`.
`command` and `root` must be absolute and `compression` is `zstd` (the default)
or `none`. This optional read adapter is not a launch readiness prerequisite:
if it is unavailable, ordinary DSH launch and readiness keep their existing
behavior and the history endpoints return only the remaining valid catalog data.
A managed exact-id fresh launch requires the configured root.
Adding `"resume":"exact-id"` to that object explicitly enables daemon-owned
history resume for the ready profile. Omitting it preserves read-only behavior.

## Trust model

The daemon binds loopback and *refuses* a non-loopback bind unless `--allow-non-loopback` is passed, because
it drives a remote shell. `GET /healthz`, `GET /sessions`, `GET /usage`,
`GET /sessions/{id}/glance`, `GET /sessions/{id}/buffer`, and
`GET /sessions/{id}/auto-resume` are intentionally open on the bind address.
Those routes can expose working directories, recent prompts, pane content,
provider usage, auto-resume state, and the server-global tmux clipboard.
Loopback blocks remote access but does not authenticate same-host principals;
the raw daemon therefore requires a trusted single-user host or an equivalent
local access-control boundary. Path-bearing
reads (`workdirs`), activity streaming, writes, and attach require the bearer token.
The provider hook ingress instead requires the session's own capability and a
direct loopback peer; it grants no operator authority. Front the daemon with the agent-console edge (which
applies its own auth) and do **not** `tailscale serve` the raw serve port; expose only the edge, tailnet-only, no funnel.
Browser WebSocket clients cannot set an `Authorization` header, so the edge must proxy the attach and inject the bearer
server-side — never put the token in the `ws://` URL/query.

## Session survival across serve restarts

The daemon starts each session as a child `tmux new-session -d`, so the tmux
server shares the caller's cgroup. Under a systemd service that means it shares the unit cgroup, and stopping or restarting
the service can kill every live session. Set `AGENT_SESSION_TMUX_SCOPE=1` to launch the tmux server inside a transient
systemd user scope (`systemd-run --user --scope`) so it lands in its own cgroup instead — a sibling of the service, so
sessions survive a daemon restart or even an explicit cgroup-wide kill. It is opt-in (the serve launcher sets it) and only
engages when a systemd `--user` manager is reachable; on any other host (no user manager, missing `systemd-run`, non-Linux)
it falls back to launching tmux directly. Pairs with `KillMode=process` on the serve unit for defense in depth.

Before binding the listener, the daemon fences reconnect for historical Codex
records that carry account bindings. Tmux exit status 1 plus a recognized
missing-server or missing-socket diagnostic is an authoritative empty snapshot:
there are no live runtimes to fence, so startup continues. Other non-success
outcomes remain unavailable and fail startup with
`codex-account-reconnect-fence-unavailable`; the daemon never treats an
unclassified inspection failure as an empty session universe.

## Federated mailbox routes

Federation is specified in [coordination](session-coordination-v1.md#cross-host-mailbox-federation-v1).
The following routes return raw versioned federation JSON; failures retain the
usual serve error envelope:

| Route | Authority | Result |
| --- | --- | --- |
| `POST /sessions/{id}/messages/remote/v1` | Current local session capability in `Authorization: Bearer` | Durable source delivery projection |
| `GET /sessions/{id}/messages/peers/v1` | Current local session capability | Ownership-filtered peer metadata |
| `GET /sessions/{id}/messages/{message_id}/delivery/v1` | Current local sender capability | Source delivery projection; no body |
| `POST /coordination/messages/receive/v1` | Operator bearer plus dedicated ingress header | Destination persistence receipt |
| `POST /sessions/{id}/console-start/v1` | Current local session capability | `agent-session.console-start.v1`, HTTP 201; see [owned child sessions](session-coordination-v1.md#owned-child-sessions-v1) |

The source submit JSON has `to_machine`, `to_session`, `body`, `idempotency_key`,
nullable `reply_to`, nullable `expires_in`, nullable `reply_revision`.
Federated routes never accept operator authority as a substitute for source
session capability. Disabled federation rejects remote submission/discovery and
receipt ingress; retained source delivery status remains queryable locally.

## Emergency shells

These authenticated routes are separate from provider sessions. The trusted edge
supplies a stable principal `owner`, matching `[a-z0-9][a-z0-9._-]{0,63}`. The
machine bearer grants operator authority to select an owner; clients must never
choose one through an untrusted body or query. Principal separation is an edge
routing contract, not an OS sandbox: principals sharing the daemon UID are
mutually trusted and share filesystem, process and tmux permissions.

GET reports status without starting a runtime. POST explicitly ensures one fixed
`ac-shell-<owner>` tmux session running `zsh -il` in the daemon user's home. Dots
and underscores in the owner are escaped as `_2e` and `_5f`. Concurrent opens are
serialized by an owner file lock; existing sessions are reused. The initial
session environment binds a unique incarnation atomically at creation, allowing
recovery after an interrupted state write. Runtime state lives under
`<state-dir>/emergency-shell`, outside provider session inventory/history.

Successful responses use the usual envelope with `data.shell`:

```json
{"schema_version":"agent-session.shell.v1","owner":"alice","status":"running","incarnation":"<uuid>","tmux_name":"ac-shell-alice"}
```

Stopped status has `incarnation: null`. DELETE requires a JSON body containing
`incarnation` and terminates only that current incarnation. A stale fence yields
`shell-incarnation-conflict`. If already stopped, DELETE returns stopped status.
A conflicting preexisting fixed tmux name fails closed rather than being adopted
or killed. `exit` ends the shell; only a later POST recreates it.

Attach requires the current incarnation and uses the existing binary PTY replay
and live output protocol. Text, lowercase special `key`/`keys`, and bounded
`resize` frames are supported. Every input and resize checks the owner/runtime;
pipe setup, snapshot capture and pipe teardown also check the incarnation.
Disconnect only releases terminal transport, preserving tmux, cwd and commands.
Provider prompt subscriptions report unsupported. Shells do not participate in
provider resume, retitle, accounts, voice or history. This interface requires
working daemon and network access; it is not an independent recovery channel.

### Display metadata and title mode

See [Session display metadata v1](session-display-metadata-v1.md) for additive
`title_mode` / `display_revision` session fields and authenticated
`POST /sessions/{exact-id}/display-metadata`. Existing title edits remain
available while pinned; model retitle is blocked until mode returns to auto.


## Lifecycle failure evidence

Authenticated create, provider-history import, resume, account-switch and delete
operations write the bounded [lifecycle journal](../runbooks/lifecycle-journal.md).
A failed runtime launch returns `ok: false` with the same typed error code and
message as the session engine. A retained failed session is available through
session reads; its presence does not turn the failed operation into success.
