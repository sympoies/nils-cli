# Serve session creation v1

Part of [Serve API v1](serve-api-v1.md). Its scope and shared rules also apply here.

## Session creation

- `POST /sessions` normally creates a fresh session from `agent`, optional `cwd`, `title`, `title_state`,
  `id`, `prompt`, `coordination_mode`, and `agent_args`, and `launch_env`. `launch_env` is an optional
  string-to-string object of public non-secret runtime switches, validated against
  `AGENT_SESSION_LAUNCH_ENV_ALLOWLIST` before creation. The default keys and bounds follow the [Public
  launch switches](../runbooks/session-lifecycle.md#public-launch-switches) contract. It is persisted and
  echoed in session views, and applied to the provider process on every runtime launch.

  Unapproved keys return `launch-env-key-refused` (HTTP 400); invalid allowlist configuration returns
  `launch-env-allowlist-invalid` (HTTP 400). `coordination_mode` accepts `advisory`, `enforce`, or `off`
  and defaults to `advisory`. A fresh create may add an advertised `agent_profile`; the id must match the
  supplied base `agent` and be ready when the request arrives. A profile whose summary reports
  `provider_resume_import_supported: true` may also be selected with `provider_resume_id`; discovery is
  then confined to that profile's provider root and never falls back to the daemon process's base root.

  The server resolves the profile's executable, provider config root, readiness command, and auto-resume
  capability; callers cannot submit or override those fields. A fresh Codex create may additionally
  provide `codex_account`; like `claude_account`, an explicit account must be a valid nickname the broker
  lists (`invalid-codex-account` / `codex-account-unknown`, both HTTP 400; `409
  codex-account-unsupported` without a broker) before anything launches, and when a prompt is also
  present, the daemon completes account binding before submitting that prompt.

  Codex prompt submission also waits for the control worker to finish loaded-thread persistence,
  reconnect/resume, and its initial usage wakeup before taking the session-record lock. Control
  registration alone does not establish readiness. Startup failure remains `409
  structured-prompt-unavailable`; the final locked incarnation and account checks still fence submission.
  A fresh, profile-free Claude create may provide `claude_account`; see the [Claude account
  broker](serve-accounts-v1.md#claude-account-broker). Each account field is rejected for another
  provider (`<provider>-account-agent-conflict`) and in provider-import mode
  (`<provider>-account-provider-resume-conflict`).

  Only `claude_account` is also rejected with an `agent_profile` (`claude-account-profile-conflict`),
  intentionally: a Claude launch profile owns the provider config root that an account binding would
  replace, while Codex credentials are injected through the app-server control plane and compose with a
  profile. When `provider_resume_id` is present (alias: `resume_id`), the daemon imports an existing
  Codex or Claude provider conversation instead: it resolves the original cwd from the selected local
  provider history, persists exact `provider_resume` metadata, and starts tmux with the canonical resume
  command.

  A capable Codex import uses the daemon-managed app-server transport so account and auto-resume controls
  remain available; unsupported or explicitly raw Codex runtimes retain the standalone resume fallback.
  In resume-id mode, omit `cwd`, `prompt`, and `agent_args`; invalid, missing, ambiguous, or unsupported
  provider ids return structured errors. Both modes accept public `workspace` and descriptive `origin`;
  see [origin and workspace](session-lineage-work-v1.md#origin-and-workspace) for nullable defaults,
  inheritance, validation, and record preservation.

  Either mode may add `lineage` (who started the session) and `work` (its program and issue references),
  validated and stored as [Session lineage and work v1](session-lineage-work-v1.md#serve-create) defines
  (`lineage-invalid`, `lineage-depth-exceeded`, `work-ref-invalid`, all HTTP 400). Without `lineage` the
  session is an operator root over HTTP. The response's `session` echoes both. For a serve-managed Codex
  session, including provider imports, `agent-session` probes bounded `codex --version` and `codex
  app-server --help` process groups. App-server transport requires Codex `>= 0.144.1` and advertised Unix
  `--listen` support.

  Exact protocol-attention authority is audited only for Codex `0.144.1`, `0.144.3` and `0.162.1`; other
  transport-compatible versions fall back to hook authority. An eligible CLI is launched as a remote TUI
  over a private short socket below an owned, non-symlinked mode-`0700` runtime root: an absolute
  `XDG_RUNTIME_DIR` when set, otherwise a platform default created mode `0700` (the daemon state
  directory's `run/`, or `/tmp/agent-session-<uid>` when the state directory is too long for a Unix
  socket). The default passes the same validation; otherwise auto mode degrades to the existing raw TUI
  and records the reason in `startup.runtime_fallback`.

  `AGENT_SESSION_CODEX_RUNTIME=raw` forces the fallback and `AGENT_SESSION_CODEX_RUNTIME=app-server`
  requires both the same capability probe and a private Unix socket. Standalone `agent-session start`
  remains raw because no serve daemon owns its control connection. The remote TUI connects through a
  private mode-`0600` WebSocket bridge to the private app-server socket. The bridge forwards frames
  unchanged, observes the exact TUI connection's structured lifecycle metadata through bounded background
  projection, and discards message content after in-memory reduction. Projection loss disables an
  existing claim without interrupting the TUI transport.

  Direct TUI thread/turn creation is launch-fenced against auto-resume before forwarding. A direct turn
  waits up to one second for transient state-lock contention before returning Busy; create-bootstrap and
  sender-owned manual-input gates remain non-blocking, and persistent contention still rejects only that
  request. This keeps an immediate post-interrupt turn from receiving a fatal transient RPC error without
  bypassing lifecycle authorization. Control-plane Enter injection (either a named Enter key or the raw
  CR/LF frame emitted by an attached terminal) already performs the same cancellation while holding the
  lifecycle lock.

  A live proxy advertises this coordination capability with a private, launch-bound, file-locked marker;
  older live proxies reject HTTP submission, while attached input disconnects without mutating
  auto-resume and requires session recreation. Immediately before the submitting tmux operation, the
  sender opens a bounded manual-input section. If ordinary proxy cancellation reports Busy, only a valid
  `turn/start` for the exact bound thread may hold that section's gate while it is forwarded. Gate
  teardown completes before the sender releases the lifecycle lock and removes the marker, preventing
  stale authorization of another lock holder.

  These markers store no prompt, terminal content, or raw thread id; malformed, expired, dead-process,
  and replacement-runtime state fails closed. The visible TUI creates the fresh thread; neither the
  bridge nor the control client synthesizes a shell or model turn. A separate daemon control connection
  reads usage and submits a continuation on that bound thread. Only a mode-`0600` SHA-256 thread binding
  is persisted, so reconnects fail closed on a mismatch and raw thread ids are never stored. The bridge
  remains with the tmux runtime across daemon restarts.

  Runtime paths are namespaced by state and launch identity; delete and launch failure validate and
  remove the app-server socket, bridge socket, and marker paths. A selected account is persisted only as
  nickname, revision, public state (`unsupported`, `unbound`, `pending`, `bound`, or `failed`), and
  applied launch id. New default-account sessions resolve and bind the current global default before
  their first prompt, then project the nickname as both `selected_account` and `effective_account` with
  `selection_source: "default_at_launch"`.

  Explicit and automatic selections use `explicit` and `auto_failover`; historical unbound records remain
  honestly unknown rather than being inferred from the current global default. On daemon reconnect or
  stopped-session resume, the new control connection re-applies that nickname before accepting input.
  Codex `account/chatgptAuthTokens/refresh` with reason `unauthorized` triggers one forced broker refresh
  and the same durable pending/bound transition. If that refresh fails before new credentials reach the
  exact runtime, the daemon restores the prior bound identity at the newer revision so a later provider
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
- Session create and PATCH requests may provide `title_state` instead of deriving semantics from the
  rendered title. Its shape is `{ "topic": string|null, "topic_source": "none"|"auto"|"user",
  "references": ["#123"], "activity": string|null }`. A `user` topic is client-owned and stable; an
  `auto` topic may be revised as the session converges; `none` requires a null topic. The daemon
  validates at most two numeric work-item references and renders the compatibility `title` as `<topic and
  references> - <activity>`, or as the only non-empty side when one side is absent. Supplying both fields
  requires an exact canonical match.

  Title-only compatibility writes remain accepted and clear `title_state`, so old clients never leave
  structured provenance attached to an unrelated title. Reads omit stale structured state if an older
  writer changed only the compatibility title. Session and glance responses advertise
  `title_state_supported: true` independently of whether that session already has structured state,
  allowing upgraded clients to migrate title-only records conservatively.

## Active-session capacity refusal

`POST /sessions`, including Console-originated creates and history imports, uses
[host-local admission](../runbooks/session-lifecycle.md#active-session-admission)
at the same atomic persistence boundary as CLI starts. The request cannot
override `admission.json` or evict a session. A full state root returns HTTP `409`:

```json
{
  "schema_version": "cli.agent-session.serve.v1",
  "ok": false,
  "error": {
    "code": "session-capacity-exceeded",
    "message": "session capacity reached (1/1); stop or delete an active session before creating another",
    "details": {
      "max_active_sessions": 1,
      "active_sessions": 1,
      "retryable": true,
      "next_action": "wait-for-capacity",
      "recovery": {
        "kind": "wait",
        "condition": "active_sessions_below_limit",
        "safe_to_retry": true
      }
    }
  }
}
```

Clients should branch on the code and numeric details, not match the message.
The failed attempt launches no tmux pane/provider and leaves no partial session
record. Retrying after capacity is released is safe under the existing creation
and idempotency rules. Resuming a stopped record also requires capacity; an
already-active resume retains its slot. Daemon restart does not reset occupancy.
Operators can inspect the read-only `agent-session diagnose --format json`
`data.admission` snapshot without sending a create request.

Console forwarding retains only validated numeric capacity and canonical recovery
fields, discarding upstream free text and extra payload. A proven pre-create
capacity refusal reconciles through the existing operation receipt as
`not_created` with `safe_to_retry: true`; reconciliation does not create again.
