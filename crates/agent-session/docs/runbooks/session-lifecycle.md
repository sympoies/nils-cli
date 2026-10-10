# Session Lifecycle Operations

This runbook owns the CLI semantics for driving one managed session after it
starts: sending input, clearing and rebinding its conversation, resuming,
deleting, reading logs, switching accounts, and the model and secret-safety
rules that apply throughout. The HTTP routes that share these code paths are
specified in [Serve API v1](../specs/serve-api-v1.md); turn phases and provider
hooks are specified in the [turn-state contract](../turn-state-contract.md).

## Send input and glance

`send` pushes input to a live session: literal text (`--text` /
`--text-stdin`) and/or repeatable named keys
(`--key enter|escape|backspace|c-c|up|down|left|shift-left|right|tab`), so
codex/claude approval prompts and terminal editing remain usable from a phone.

- Text is pasted with bracketed-paste markers (`tmux paste-buffer -p`), so a
  multi-line message stays one prompt with its line breaks.
- Text without `--key enter`, or with other keys, only types; `--key` alone
  presses keys as before.
- `send` bumps `updated_at`, so `list` orders by real control-plane activity.
- `send` is not idempotent: keystrokes are delivered before the command
  returns, so a retry after a mid-delivery failure can re-send. Callers that
  auto-retry should account for this.

`--text`/`--text-stdin` followed by exactly `--key enter` is a prompt
submission. `send` reads the pane back and reports
`data.submission = { outcome, enter_presses }`:

- `submitted` means the text left the Claude Code or Codex composer.
- `queued` means Claude Code queued it behind a running turn ("Press up to edit
  queued messages").
- When the text is still in the composer, `send` presses Enter once more,
  unless the session is `needs_input` or the pane shows a dialog.
- If the text is still there after that, `send` fails with `send-submit-stuck`
  (`error.details.outcome = "stuck"`) and leaves the text in place. The text
  has already reached the pane, so do not resend it: `glance` the session or
  clear the composer first.
- `unverified` means the pane could not prove either way: another provider, an
  unrecognized layout, a dialog, or a first line that looks like a numbered
  choice (`1. …`).

While a session is `needs_input`, its pane belongs to a provider approval or
question dialog, not to a prompt box. The routes that write to the pane refuse
input carrying literal text with `agent-blocked` before anything reaches the
terminal. Special keys stay admitted, because that is how the dialog gets
answered, and `send --allow-blocked` is the deliberate opt-in for typing into
the dialog's own field. Routes that do not touch the pane, such as a Codex
app-server prompt, are unaffected. This is a safety default rather than an
authorization boundary: the attach socket remains an interactive terminal. The
[Serve API v1 blocked-input contract](../specs/serve-api-v1.md) has the
per-route table and the exact codes.

`glance` returns the recent pane tail plus live status as a JSON contract for
dashboard tiles (cheaper than a full attach).

## Clear a conversation in place

`clear <id> --expect-idle --format json` clears an idle interactive
conversation in place: Codex uses native `/new` through its managed app-server
TUI and Claude Code uses `/clear` plus the paired `agent-hook` clear receipt.

- It preserves the managed session/runtime identity, records the new exact
  provider resume ID, and resets the turn snapshot.
- It returns `old_provider_session_id`, `new_provider_session_id`, `changed`,
  and `support`.
- Idle admission is mandatory with or without `--expect-idle`; busy, blocked,
  starting, unknown, stopped, and unsupported sessions are refused.
- `--timeout` bounds clear confirmation (default 15 seconds). A confirmation
  timeout is not success and must not be blindly retried: inspect the native
  prompt/dialog and use `rebind <id> --format json`.

## Rebind a stale provider binding

`rebind` repairs stale provider bindings without clearing again.

- Codex requires a provider-verified idle thread and either a runtime-bound
  native observation or one unambiguous loaded primary thread; ambiguous loaded
  threads are refused rather than selected from history.
- Codex also requires a live proxy advertising conversation-rebind support. An
  older already-running proxy is refused before terminal input: stop and resume
  the same managed session through the upgraded installation first.
- Claude requires a runtime-bound clear hook receipt and an idle snapshot, and
  the matching `agent-hook` binary on the launch and resume PATH.
- Rebinding older proxy process state or selecting among ambiguous unobserved
  threads is unsafe; neither is inferred from transcript order.

Raw provider IDs remain private lifecycle metadata; activity events continue to
use projected identities. Native primary clears are also observed
automatically; auxiliary threads and compaction do not rebind the managed
conversation. The next provider turn and a later `resume` use the new ID. The
[turn-state contract](../turn-state-contract.md#conversation-transitions)
defines the observation and recovery rules.

## Resume a stopped runtime

`resume` recreates a missing tmux runtime only when the session has exact
provider resume metadata; it never resumes the latest provider conversation
implicitly.

- Runtime metadata is persisted before launch so hooks see the new generation,
  and the immutable tmux session/pane identity is persisted before a successful
  start or resume returns.
- Resume first proves the current and every retained prior launch identity
  stopped, so a surviving provider process cannot be hidden by a new runtime
  generation.
- An older stopped record without that proof returns the same non-retryable
  manual-verification action as deletion; only a generation durably marked as
  never launched can resume without a runtime identity.
- If tmux launch fails, the prior runtime and activity snapshot are restored
  only after any possibly launched replacement is verified stopped. An
  unverified replacement remains the current discoverable generation.

Retired orchestration registries and sidecars are ignored. Session records
retain unknown metadata through ordinary rewrites without projecting it or
restoring mode authority. Retirement does not migrate or delete durable state.

## Delete a session

`delete` removes provider runtime files and session metadata only after bounded
checks verify the recorded runtime is stopped.

Before killing a live runtime, it:

1. inspects only the managed `0.0` pane;
2. captures its immutable tmux session/pane ids and process boundary;
3. validates the runtime's `AGENT_SESSION_*` ownership markers;
4. persists that identity for retry;
5. uses one tmux-server conditional command to kill only if the captured
   session, pane, and pane PID still match.

If the managed pane is replaced, it durably retains every unresolved observed
process identity before proceeding. It then targets only the captured tmux
session id.

On Linux:

- It snapshots each verified process-session member by PID and start time and
  requires the pane to be in a leaf cgroup-v2 `tmux-spawn-*.scope`.
- It pins the process sets observed before and during a bounded stabilization
  window after freezing that cgroup, revalidates the pane membership and cgroup
  inode, conditionally kills the exact tmux identity, and invokes `cgroup.kill`
  while the boundary remains frozen.
- Members present at either verified boundary cannot survive by changing
  process session or by later cgroup migration.
- The cgroup identity also records the Linux boot id, so startup recovery never
  applies an old PID/start-time or cgroup identity to a different boot.
- A durable ownership marker lets a delete retry or a restarted server thaw only
  a scope that deletion changed from unfrozen to frozen; a scope that was
  already frozen remains frozen.
- Without that distinct leaf cgroup, Linux deletion fails closed before
  mutating tmux.

Other Unix platforms retain verify-only handling for the pane process-group
boundary.

Cleanup and retry rules:

- Cleanup requires tmux and every retained process boundary to be gone.
- A retry can finish cleanup without another kill only when all persisted
  identities verify stopped.
- Live records created by an older version can be upgraded from their ownership
  markers.
- A stopped pre-upgrade record without a provable launch identity remains
  retained and returns `runtime-identity-unavailable`, `retryable: false`, and
  `action: manual-runtime-verification-required`; an operator must verify its
  runtime manually before removing that state.
- Kill failures, ambiguous tmux errors, ownership mismatches, and surviving
  processes retain all session state.
- When tmux returns a successful but blank identity probe for a stale session,
  deletion confirms the exact session is absent and still requires every
  persisted process boundary to verify stopped before removing metadata.

Human success output reports the verified stopped state; the v1 JSON
`killed: true` field remains stable for successful deletion.

## Read logs

For a stopped session without a one-shot run log, `agent-session logs <id>`
falls back to the private, tail-capped startup diagnostic. Codex sessions retain
that diagnostic after a startup failure or non-zero provider-client exit; a
clean exit after readiness discards it.

## Start DSH sessions

DSH panes start only through a server-owned `serve` launch profile with base
agent `dsh`, which launches the profile's `agent_bin` with no implicit
subcommand. A profile-less `start --agent dsh` is refused, and one-shot `run`
mode is codex/claude only. Launch profiles are specified in
[Serve API v1](../specs/serve-api-v1.md#launch-profiles).

## Switch provider accounts

With `AGENT_SESSION_CLAUDE_ACCOUNT_BROKER` configured, each daemon-created
Claude session binds to a chosen account: the broker materializes a
per-account `CLAUDE_CONFIG_DIR`, the binding survives resume, and a switch
relaunches the session with `--resume` in the new account directory. See the
[Claude account broker](../specs/serve-api-v1.md#claude-account-broker).

`account switch <id> --account <nickname>` is the local, owner-run CLI for the
same switch as serve's `PUT /sessions/{id}/account`; both call one shared code
path and need no serve token.

- Claude queues the next account. When the session is idle it preflights that
  account, stops the session and resumes the same conversation under it; while
  busy the switch stays queued.
- Run from inside the session's own tmux session, a Claude switch only queues.
- Codex binds the account for the next prompt: the CLI durably queues it and
  the serve daemon's control connection applies it at the idle boundary, so the
  next prompt is fenced until then. A Codex switch needs a running serve daemon
  to apply; re-selecting the current account cancels a queued switch.
- `--expected-incarnation` fences the switch to one runtime and defaults to the
  current one.
- JSON output returns the provider's `codex_account` or `claude_account` view
  with `session_incarnation`.
- Failures keep serve's typed codes (`claude-account-switch-refused`,
  `<provider>-account-session-incarnation-conflict`,
  `<provider>-account-unknown`, `<provider>-account-unsupported`).

`account show <id>` returns the current account and any queued `next` one. Both
commands, and `resume`, use the provider's account broker variable from the
caller's environment, or else the brokers serve recorded in the private state
dir at startup; with neither, `show` reports the account as unsupported.

After stopping, a Claude switch retires the stopped runtime's coordination
incarnation before resuming. If the resume still fails, it returns
`claude-account-switch-resume-failed` with the `agent-session resume <id>`
recovery command; the session is stopped with the account queued.

On macOS, a revoked broker from the same boot may be retired or replaced only
when all of these hold:

- its persisted identity exactly matches the prior record;
- both the managed tmux name and numeric target were absent before launch;
- a complete process-group probe is empty.

The group is checked again under the coordination lock before replacement.
Live, unavailable, or mismatched evidence still refuses recovery.

## Model and effort settings

Session creation retains `model_settings` in the session record, scoped to the
runtime launch ID. `list --format json`, HTTP session responses, and board
records expose nullable `model` and `reasoning_effort` fields. Missing values
are unknown, including old records without explicit launch settings; consumers
must not infer values from accounts, profiles, or the title-generation model.

The launch projection recognizes Claude `--model` / `--effort` and Codex
`-m` / `--model`, `-c` / `--config` model and `model_reasoning_effort`
overrides, including local Codex `--oss` models. A local Claude-compatible
launcher can supply the same explicit flags. Provider config files,
wrapper-internal settings, and implicit defaults are unknown until the provider
reports them.

Matching provider hooks can update bounded model/effort metadata:

- Claude `SessionStart` model and structured `effort.level` metadata are
  projected by `agent-hook` activity rules without forwarding prompts, paths,
  or raw provider identities.
- Matching tool/stop hooks update effort and read at most 64 KiB from the
  trusted primary transcript for the actual model. A model switch becomes
  visible after the provider emits that evidence.
- Both helpers must be updated; older helpers safely leave unavailable fields
  unknown.

Managed Codex app-server thread responses report resolved defaults, and
successful `turn/start` responses confirm later explicit model/effort changes.
Observations must match the active runtime and provider session; auxiliary
sessions cannot replace primary settings. A changed model without an effort
observation clears the previous effort. Labels resembling credentials or paths
are omitted.

## Keep secrets out of commands

- Prompts are stored under the local agent-session state directory and are not
  printed in command output.
- For sensitive prompts, prefer interactive `start`; one-shot `run` may need to
  pass the prompt through the underlying agent process command line depending
  on that agent's CLI capabilities.
- `send` routes literal text through a private (0600) buffer file loaded into
  tmux, so it never appears in the tmux command line or command output. The
  JSON contract reports only `sent_text` (a boolean) and the special-key names,
  never the text itself.
- For secrets, prefer `--text-stdin`: `--text <value>` still places the literal
  in agent-session's own process arguments (visible in `ps` to same-user
  processes), exactly as the existing `--prompt` flag does.
- Values passed with `--agent-arg` are persisted in the private session record
  so durable resume can recreate the same provider invocation. Do not put
  secrets in provider arguments.
- For Claude sessions, provider identity/resume flags such as `--session-id`,
  `--resume`/`-r`, `--continue`/`-c`, `--fork-session`, and `--from-pr` are
  reserved for agent-session so the stored resume identity stays exact.
