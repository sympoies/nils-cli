# Serve observation v1

Part of [Serve API v1](serve-api-v1.md). Its scope and shared rules also apply here.

## Health and session projections

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

- `GET /healthz`, `GET /sessions`, `GET /sessions/{id}/glance?tail=N` — reads, open on loopback. `GET
  /sessions` additively reports `data.observed_at`, sampled from daemon time after the returned session
  state is assembled, plus `data.agent_profiles` containing only ready server-owned `{ id, label, agent,
  provider_resume_import_supported, history_resume_supported }` launch-profile summaries.
  `data.capabilities.profile_resume_import` advertises support for selecting one of those safe ids during
  provider import; executable paths, configuration roots, readiness commands, and environment remain
  private.

  `data.capabilities.managed_resume_command` remains false until an unqualified CLI resume can revalidate
  the daemon's active profile registry and readiness contract; consumers must copy the provider session
  ID during that skew. `data.capabilities.managed_account_handoff` advertises that this daemon
  understands the managed-handoff protocol; it does not make every session eligible. A session-level
  `capabilities` array contains `agent-session.codex-managed-account-handoff.v1` only for a live Codex
  app-server runtime whose launch-bound capability probe succeeded. Raw Codex tmux sessions and all
  Claude sessions omit that session capability.

  Consumers must require both the global protocol advertisement and the per-session capability before
  presenting managed handoff controls. `data.capabilities.codex_account_switch` and
  `data.capabilities.claude_account_switch` are each `true` only while that provider's broker
  (`AGENT_SESSION_CODEX_ACCOUNT_BROKER` / `AGENT_SESSION_CLAUDE_ACCOUNT_BROKER`) is configured; see
  [Account brokers](serve-accounts-v1.md#account-brokers). `codex_account_switch` is additive: older
  daemons omit it, and a Codex session's own `codex_account.supported` remains the per-session authority.

  Sessions report `running`, `stopped`, or `unknown` live status plus a boolean `resumable` field and
  best-effort `repo_name` derived from the recorded `cwd`. New interactive records also expose optional
  `runtime_started_at`, `turn_state`, `last_prompt`, `last_prompt_state`, `last_prompt_continuity`, and
  `startup`; a profiled session also exposes its safe `agent_profile` id. When a known profile drift
  would make a stopped session fail resume, the daemon sets `resumable: false` plus one bounded
  `resume_blocked_reason` code. Old records and daemons omit the additive fields.

  `data.capabilities.last_prompt` advertises the list `last_prompt` preview: the most recent user prompt
  for a running Codex/Claude session, resolved from the exact provider transcript so it reflects prompts
  submitted through any input path (web console, SSH/Termius, or raw `tmux attach`). Codex Goal creation
  and objective edits also update this preview as `Goal: <objective>` in transcript order; later user
  messages replace it. Goal status/usage updates and internal continuation context do not revive an older
  instruction. Goal observation is preview-only and never emits `prompt_submitted` or acknowledges a
  broker input delivery.

  Inline string-valued `image_url` content is elided before the 256 KiB line-buffer bound, preserving
  surrounding user text without retaining image payloads in that buffer. For Codex records containing an
  actual image, paired image name/path wrapper lines are removed from the list preview so they cannot
  hide the user question; submission-event text stays unchanged. If cold recovery starts mid-history, the
  first existing Goal snapshot establishes its comparison baseline without replacing a newer prompt,
  unless its event timestamp identifies a new Goal creation. An objective edit cannot be inferred from an
  isolated existing snapshot; subsequent objective changes are observed normally.

  The 64 KiB per-read and 16-read catch-up budgets remain in force; oversized non-image records still
  fail closed. On first discovery the daemon opens an append tail and queues one at-most-64-MiB cold
  recovery outside the list-response path. Cold recovery and append catch-up are single-flight per
  session and share a daemon-wide concurrency bound. Eligible running Codex/Claude sessions add
  `last_prompt_state` with one of `current`, `pending`, or `unavailable`. `current` may include
  `last_prompt`; `current` without it authoritatively means the caught-up transcript has no eligible user
  prompt.

  `pending` means exact-source discovery, cold recovery, or known append catch-up is in progress and
  omits the preview rather than reporting a stale cached value. `unavailable` means the exact source
  cannot currently be used or its continuity was invalidated, and also omits the preview. After
  continuity invalidation, every caller continues to see `unavailable` until one response can expose an
  authoritative `current` projection. The response-only opaque `last_prompt_continuity` token is 16-128
  URL-safe ASCII characters, is present with eligible states, and rotates whenever exact transcript
  continuity is lost, including across daemon restarts.

  Consumers may retain a pending preview only when this token and the runtime identity both match.
  Sessions without enough runtime/provider identity to be eligible omit both fields. Later list polls use
  a bounded metadata/continuity check, retain the newest caught-up preview in process memory, and avoid
  recurring cold scans when the stable registry is at capacity. Rotation, truncation, or identity drift
  invalidates that memory and forces exact rediscovery. The text is returned in the response only and is
  never logged or persisted by the daemon; the daemon never guesses a transcript match.

  `startup` is the metadata-only `agent-session.startup.v1` projection shared by create, list, and glance
  responses. Its state is `starting`, `ready`, or `failed`; its bounded stage is `record`, `tmux`,
  `runtime`, `app_server`, `proxy`, `provider_client`, or `initial_connection`. A Codex session whose
  automatic runtime selection kept the raw TUI adds `runtime_fallback` with one allowlisted reason:
  `codex-unavailable`, `codex-version-unrecognized`, `codex-version-too-old`,
  `codex-app-server-transport-unavailable`, `codex-app-server-runtime-dir-unavailable`,
  `codex-app-server-runtime-dir-unsafe`, `codex-app-server-socket-path-too-long`, or
  `codex-app-server-runtime-unavailable`.

  It is absent for an app-server runtime and for an explicit `AGENT_SESSION_CODEX_RUNTIME=raw`. Failed
  projections add an RFC 3339 `occurred_at` captured from the private failure marker, boolean
  `retry_safe`, one reviewed message, and one allowlisted code: `runtime-helper-unavailable`,
  `agent-binary-unavailable`, `working-directory-unavailable`, `terminal-runtime-create-failed`,
  `app-server-start-failed`, `proxy-start-failed`, `provider-client-exited`,
  `provider-configuration-rejected`, `startup-trust-required`,
  `startup-prompt-required`, `startup-timeout`, `startup-exited`, or
  `claude-account-switch-resume-failed`, or `claude-account-switch-cleanup-incomplete`.

  A failure whose cleanup could not finish adds a bounded `cleanup` object with `state` of `pending` or
  `blocked` and one allowlisted `reason`: `session_still_running`, `process_boundary_live`,
  `runtime_identity_changed`, `runtime_identity_unavailable`, `termination_failed`,
  `termination_timeout`, `verification_failed`, `cleanup_unavailable`, or `unknown`. `cleanup` is absent
  when cleanup completed or was never attempted, so a projection that omits it carries no caveat. Cleanup
  is strictly secondary: it never replaces the primary startup failure, because the original create/start
  error is what explains the failure and a termination error would hide it.

  When cleanup does not complete, the session record is deliberately retained rather than removed — a
  live boundary with no record would be unreachable from the Console — and the runtime is not marked
  never-launched. Managed launchers retain only bounded stage/failure markers in the record and keep
  stderr in a private, tail-capped local diagnostic file for startup failures and non-zero Codex
  provider-client exits after readiness; clean exits discard it. The local `agent-session logs <id>`
  command can read that diagnostic, but it is never copied into the session projection.

  Fresh create/start observes startup for up to five seconds (the existing capture-timeout override
  also bounds this observation). An observed stop before provider readiness returns HTTP 500 with
  `error.code: "start-failed"`, rather than a successful stopped session. Its `error.details` contains
  the retained `session_id`, failed `startup` projection, `reason: "runtime-exited-during-startup"`,
  nullable numeric `provider_exit_code`, optional redacted `stderr_tail` capped at 4096 bytes, boolean
  `retryable` and `safe_to_retry`, `next_action`, `recovery.action`, and `cleanup`.

  Retry is allowed only after verifying that the recorded runtime boundary has stopped. Missing exit or
  diagnostic evidence remains null/absent; it is never invented. Startup timeout or an unobservable
  runtime does not prove failure or authorize retry.

  A recognized folder-trust screen returns the same `start-failed` envelope with
  `reason: "startup-trust-required"` and `next_action: "configure_provider_trust"`; recognized
  sign-in/setup screens use `reason: "startup-prompt-required"` and `next_action: "configure_provider"`.
  These prompts are never answered automatically. Failed-launch cleanup is attempted and reported;
  incomplete cleanup uses `next_action: "inspect_session"` and always disables retry. The failure
  response includes a bounded, redacted diagnostic only; the durable session projection remains
  metadata-only.

  Raw argv, environment, provider responses, stderr, prompts, and filesystem paths are never copied into
  the projection.

  A record that reached `ready` keeps that state after an ordinary later stop, so consumers must not
  relabel normal session termination as startup failure. A fresh managed Codex client that exits nonzero
  before binding its first provider thread reports `provider-client-exited`, even if an earlier view
  reported `ready` from the initial proxy connection. A resume starts a fresh startup lifecycle for its
  new runtime generation; synchronous launch rollback restores the prior projection and private
  diagnostic artifacts. A leftover resume backup from an interrupted process blocks another resume before
  mutation so the only copy of prior diagnostic state is not silently discarded.
- `GET /usage` — read-only provider usage report, open on loopback. The serve envelope contains
  `data.usage.schema_version: "agent-session.usage.v1"` and provider entries for Codex and Claude.
  Provider readers are bounded by `AGENT_SESSION_USAGE_TIMEOUT_MS` (default 45000). The Claude reader
  forwards that budget to its nested probe with five seconds reserved before the outer hard deadline when
  the budget is at least six seconds; shorter budgets use a one-second inner minimum. A positive
  `CLAUDE_PROMPT_SEGMENT_CLAUDE_TIMEOUT_SECONDS` override is kept when it fits and clamped when it
  exceeds that inner budget. Timed-out helpers are killed as a process group before their output pipes
  are read.

  Provider readers preserve partial success, preserve reset timestamps as `reset_at_epoch` epoch seconds
  plus textual `reset_at` when supplied by the helper, and redact tokens, local auth paths, and private
  account identifiers from scoped error messages. Failed providers may include the additive
  provider-neutral `reason_code` contract (`auth_required`, `auth_expired`, `billing_past_due`,
  `subscription_inactive`, `organization_disabled`, `permission_denied`, `rate_limited`,
  `service_unavailable`, `timeout`, or `unknown`) copied only from the helpers' allowlisted structured
  field.

  The authenticated, cached, per-account successor is `GET /usage/v1`, specified under [Provider usage
  and resets](serve-usage-v1.md#provider-usage-and-resets).
- `GET /repos/remote-url?cwd=...` — authenticated repository lookup. `cwd` is
  required. The ordinary serve envelope returns `data.url` as a normalized
  credential-free HTTPS URL for any parseable Git origin host, or `null` when
  the directory has no supported remote.
- `GET /sessions/{id}/buffer` — open on loopback. Returns the tmux server's
  latest global clipboard buffer after using `id` only to verify that the
  requested session exists. The buffer is not scoped to that session.
- `GET /workdirs?q=...&limit=N` — authenticated read; searches only the default operator roots (`$HOME/Project` and
  `$HOME/.config`) with bounded depth, count, and elapsed-time limits. Add `git_only=true&exclude_worktrees=true` for
  the curated project picker: only primary git working trees are returned, ordered by most-recent session cwd usage
  (`last_used`) and then name/path.

## Activity stream

- `GET /activity/events` — authenticated metadata-only SSE for activity snapshots and heartbeats. Events
  carry a daemon-boot `stream_id` and increasing `sequence`; `Last-Event-ID` enables
  count-and-byte-bounded replay, while stale/foreign cursors and lagged consumers receive a reset.
  Concurrent subscribers are daemon-capped and saturation returns a stable polling-fallback error.
  Provider hooks only update durable local activity files; a daemon filesystem watcher publishes changes
  through bounded nonblocking queues.

  Payloads and SSE frames are serialized once and shared; an oversized snapshot emits a transition-only
  content-free `oversized_snapshot` reset that requires immediate polling rather than entering replay or
  broadcast retention. Notification storms converge through a trailing quiet debounce and refresh starts
  spaced by an explicit minimum cadence. Backend rescan flags force a full refresh; sessions-root loss
  re-arms a replacement recursive watcher or degrades the stream. Watcher or snapshot-source failures
  send existing streams one reset before closing, stop heartbeats, and make new stream requests return
  the polling-fallback error.

  The stream and `/sessions` share one snapshot source; degraded resets use unique sequences while
  retaining the last successful snapshot observation anchor, and typed projection omits absent nested
  leaves while preserving session-level `turn_state: null`. The existing `/sessions` read remains the
  old-peer and gap-reconciliation path. The exact wire/privacy contract is
  [activity-stream-v1](activity-stream-v1.md).
