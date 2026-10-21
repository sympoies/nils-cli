# Serve terminal transport v1

Part of [Serve API v1](serve-api-v1.md). Its scope and shared rules also apply here.

## Terminal input and attach

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
- `GET /sessions/{id}/attach` — a WebSocket PTY attach: a `capture-pane` snapshot then a live byte stream
  from one daemon-owned `tmux pipe-pane` broker per session (binary frames, renderable by xterm.js).
  Concurrent clients fan out from the same bounded in-memory stream; disconnecting one client leaves the
  others live, while a lagging client is disconnected so it cannot stall tmux or other clients. The
  broker uses a private ephemeral FIFO and retains no interactive-session terminal bytes after the final
  client disconnects. Snapshot capture drains live output into a bounded handoff buffer and performs one
  bounded fresh-snapshot recovery if that buffer overflows.

  After handoff, a supervised per-client pump keeps draining broker output independently of provider
  discovery, input, and resize work; a normal broker close drains already accepted frames under the
  WebSocket send bound, while lag/error teardown remains immediate. The client sends JSON control frames
  `{ "text": "...", "key": "enter", "keys": ["c-c"], "resize": { "cols": 80, "rows": 24 } }`.
  Token-gated; disconnect leaves the tmux session alive. The named key `shift-left` uses tmux `S-Left`,
  honoring the pane's negotiated keyboard mode. For older clients, an exact `text` frame containing
  `ESC[1;2D` also means Shift+Left; the sequence inside other text or bracketed-paste markers remains
  literal.

  Both forms use the same serialized session input and runtime fencing as other keys. Concurrent clients
  share the pane geometry; resize sequences are serialized and the last completed resize wins. A client
  may opt into authoritative Codex/Claude prompt events by sending `{ "subscribe": ["provider-prompt.v1"]
  }`. For a known, resumed, imported, or reconnected provider session, the daemon baselines the exact
  provider transcript at EOF.

  When a generation-1 fresh Codex/Claude runtime is still establishing its exact provider identity or
  transcript, the connection instead keeps a bounded, cancellable resolver alive, reloads the same
  launch's session metadata, and opens that fresh transcript from its beginning so the first prompt is
  not lost. Codex `UserPromptSubmit` hook metadata supplies the exact runtime-bound session identity; the
  pending attach path never promotes cwd/time history scans into beginning-of-transcript authority.
  Transcript discovery is shared by runtime across attach clients, is owned by the daemon across waiter
  cancellation, uses bounded exponential backoff, and admits at most four concurrent history scans.

  Active slots are never capacity-evicted; obsolete runtime keys and deleted sessions are evicted, and a
  fixed daemon-local entry cap bounds unrelated session churn. Reconnects baseline the revalidated cached
  exact source at EOF. Passive list/glance reads never persist heuristic Codex history into a live
  generation-1 runtime, and fresh Codex byte-zero recovery admits only `codex-user-prompt-submit-hook`
  identity. Explicit stopped-session resume may recover older provider history while holding the same
  per-session record lock for its complete resume/rollback transition.

  It replies with an `agent-session.attach.v1` `capability` text frame once resolution finishes, and only
  after that acknowledgement emits `prompt_submitted` events as bounded text frames; terminal
  snapshot/live output remains binary. The normative supported acknowledgement is:

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

  `provider` is `"codex"` or `"claude"` when supported and `null` otherwise; an unsupported provider,
  unresolved exact transcript, unsafe transcript path, exhausted discovery budget, or expired
  fresh-runtime resolution returns the same object with `supported:false`. The fresh-runtime resolver is
  restricted to the original generation-1 launch identity: Codex must have had no provider identity when
  the client subscribed, and Claude must carry the daemon-generated `claude-explicit-session-id` capture
  method. Imported, resumed, replaced, and later-generation runtimes never enter the
  beginning-of-transcript path, so reconnect retains EOF/no-history behavior.

  While resolution is pending, terminal bytes, input, resize, and broker fanout continue independently;
  consumers may activate their bounded local fallback before the eventual acknowledgement. Clients that
  do not subscribe receive no event text frames. The normative event is:

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
