# Session Lifecycle Operations

This runbook owns the CLI semantics for driving one managed session after it
starts: sending input, clearing and rebinding its conversation, resuming,
deleting, reading logs, switching accounts, and the model and secret-safety
rules that apply throughout. The HTTP routes that share these code paths are
specified in [Serve API v1](../specs/serve-api-v1.md); turn phases and provider
hooks are specified in the [turn-state contract](../turn-state-contract.md).

## Active-session admission

To limit concurrent sessions sharing a local state root, put `admission.json`
in the directory selected by `--state-dir` or `AGENT_SESSION_STATE_DIR`:

```json
{"max_active_sessions": 1}
```

The optional value is a non-negative integer: `0` refuses all new admissions;
missing file, omitted key, or `null` keeps the default unlimited behavior.
The setting is read on each admission, so CLI and daemon callers using the same
state root share the effective policy without restarting the daemon. Capacity
admission is independent of advisory work-context claims and applies across
coordination modes. Keep the
state root operator-owned and replace the settings file atomically when editing
it. Lowering the limit never evicts an existing session.

Admission is serialized with initial session-record persistence before tmux or
provider launch. It covers interactive `start`, one-shot `run`, HTTP/Console
creates, history imports creating a new session, and stopped-record reactivation
through resume/restart. Resuming an already-running session uses its existing
slot. Existing owner, incarnation, lifecycle, and idempotency fences still apply.

A starting session's existing lifecycle lock and persisted record reserve a slot
until creation finishes. Running sessions continue to occupy capacity across
CLI exits and daemon restarts. Stopped/deleted sessions and failed or cancelled
creates without a live runtime release capacity. An uncertain runtime remains
counted until it is verified stopped; admission never kills another session.

Read the effective policy and current occupancy with:

```sh
agent-session diagnose --format json
```

The additive `data.admission` object contains only `max_active_sessions`
(integer or `null`) and `active_sessions` (non-negative integer). Text diagnostics
show the same values. This read-only snapshot does not create a state root,
normalize registry state, change session records, or launch a provider; it may
probe tmux liveness. Occupancy can change immediately after the snapshot.

At capacity, CLI commands exit `1` with `error.code` set to
`session-capacity-exceeded`. The refusal's `error.details` contains
`max_active_sessions`, `active_sessions`, `retryable: true`,
`next_action: "wait-for-capacity"`, and bounded `recovery` metadata with
`safe_to_retry: true`; the human message tells the operator
to stop or delete an active session. No refused create leaves a session directory
or starts tmux/provider work. HTTP callers receive the same code and details with
HTTP `409`; see [session creation v1](../specs/serve-session-create-v1.md).
An unreadable or invalid settings document refuses admission with
`session-admission-config-invalid` instead of silently disabling the limit.

Occupancy read or validation failures refuse admission with the path-free
`session-admission-state-unavailable` error. They never treat unavailable state
as empty capacity or expose a local record path.

## Start origin and workspace

`agent-session start --agent codex --workspace example-workspace` selects a
public workspace slug; omit `--workspace` to inherit the known managed
parent's workspace. A plain-shell start is unassigned by default. Origin
captures the managed caller's session id and machine automatically, including
console starts relayed to another host.

Tag an existing long-running session without restarting it:

```bash
agent-session workspace set SESSION_ID example-workspace --format json
agent-session list --format json
agent-session command SESSION_ID --format json
```

The list/command session view exposes `origin: {"parent": null}` and
`workspace: null` when unassigned. Use only public, non-secret slugs: 1–64
lowercase ASCII letters/digits, single hyphen or dot separators, alphanumeric
ends. Invalid values return `workspace-invalid` with a fix-it command. Resume
and account handoff preserve both fields. See the
[session record contract](../specs/session-lineage-work-v1.md#origin-and-workspace)
for cross-host inheritance and nullable JSON details.

## Send input and glance

`send` pushes input to a live session: literal text (`--text` /
`--text-stdin`) and/or repeatable named or single printable keys
(`--key enter|escape|backspace|c-c|up|down|left|shift-left|right|tab`, or `--key s`), so
codex/claude approval prompts and terminal editing remain usable from a phone.

- Text is pasted with bracketed-paste markers (`tmux paste-buffer -p`), so a
  multi-line message stays one prompt with its line breaks.
- Text without `--key enter`, or with other keys, only types; `--key` alone
  presses keys as before.
- `send` bumps `updated_at`, so `list` orders by real control-plane activity.
- Unguarded `send` is not idempotent: keystrokes are delivered before the command
  returns, so a retry after a mid-delivery failure can re-send. Callers that
  auto-retry should account for this.

A printable key always requires verified idle activity, even without
`--require-idle`. Busy or unknown sessions are refused before typing. Named
keys keep their existing dialog/interrupt behavior.

For a session-only model change, inspect the provider's picker with `glance`,
navigate with named keys, and type the session-only shortcut it advertises:

```bash
agent-session glance <id> --tail 40
agent-session send <id> --require-idle --text /model --key enter
agent-session glance <id> --tail 40
# Navigate the displayed model/effort choices in separate guarded sends.
agent-session send <id> --require-idle --key down
# Only when the displayed picker advertises s for this session only:
agent-session send <id> --key s
agent-session glance <id> --tail 40
```

Verify the resulting model and effort from the final provider display. In
pickers that advertise `s` for this session only, `enter` may instead save a
default; use it only when that persistent change is intended. If the provider
version, picker shortcut, scope, or resulting settings cannot be verified, stop
without confirming. This is the printable-key alternative for model switching;
there is no automatic `set-model` command or provider-version support claim.
For current Claude picker scope behavior, see the provider's
[model configuration documentation](https://code.claude.com/docs/en/model-config#setting-your-model).

For opt-in continuation guards and persisted at-most-once keys, use the
[Send v1 contract](../specs/send-v1.md). Guarded submissions verify exact input
and press Enter at most once; the retry behavior below applies to unguarded
sends.

`--text`/`--text-stdin` followed by exactly `--key enter` is a prompt
submission. `send` reads the pane back and reports
`data.submission = { outcome, enter_presses }`:

- `submitted` means the text left the Claude Code or Codex composer, or an
  accepted Enter was followed by verified absence of the exact terminal runtime
  (for example, after `/exit`). A failed pane read alone does not confirm exit.
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
terminal. Named special keys stay admitted, because that is how the dialog gets
answered, and `send --allow-blocked` is the deliberate opt-in for typing into
the dialog's own field. Printable `--key` values remain refused in this phase,
even with `--allow-blocked`, because they always require idle admission.
Routes that do not touch the pane, such as a Codex
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

## Unexpected provider exit

A provider quit or interrupt without a successful control-plane stop leaves the
managed session and its exact conversation available for explicit recovery.
`agent-session list` shows the exit reason and a
`Resume: agent-session resume SESSION_ID` action when resume metadata is valid.
`agent-session command SESSION_ID --format json` includes the same structured
metadata; text `command` keeps its existing attach-command output. There is no automatic relaunch; repeated reads cannot create a restart
loop. A provider-side quit may be intentional, so recovery requires this action.

Session JSON adds `runtime_exit` for the current stopped runtime generation:
`code` and `signal` (nullable), `reason`, `unexpected`, `observed_at`, `message`,
and optional `recovery`. Reasons are `provider-exited`, `app-server-close`,
`runtime-disappeared`, and `runtime-stopped`. The recovery object carries
`label: "Resume"`, the CLI `command`, and the existing HTTP action
`method: "POST"`, `path: "/sessions/SESSION_ID/resume"`. API consumers can
render the reason beside that action. Existing Console rendering is unchanged.
A successful explicit stop suppresses this unexpected-exit recovery offer.

The launcher supplies the observed shell status (for example, 130 after a
SIGINT handler exits). A shell status alone cannot distinguish a signal from an
explicit exit with the same status, so the signal stays null unless known.
A known process code or signal takes precedence over earlier app-server control
closure; `app-server-close` is the fallback when process status is unavailable.
Missing evidence stays null; provider output is never copied into the public
reason. The board advertises `runtime_exit.v1` and carries the same bounded
metadata. A running or replaced generation omits the prior exit.

Resume still uses all existing locking, exact identity, profile, and stopped
runtime checks below. A recovery offer is not proof that those checks will pass.

## Resume a stopped runtime

`resume` recreates a missing tmux runtime only when the session has exact
provider resume metadata; it never resumes the latest provider conversation
implicitly.

- Tmux probe timeouts and server-shutdown uncertainty are retried for up to two
  seconds while holding the session record lock. A confirmed running runtime is
  a no-op; confirmed absence proceeds through the stopped-runtime checks below.
  Persistent transient uncertainty returns `session-status-unknown` with
  `retryable: true` and `next_action: retry-resume`. Other probe failures, such
  as a missing executable or denied socket access, return the same code with
  `retryable: false` and `next_action: inspect-tmux`. Neither launches a replacement.
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
- A provider that just exited cleanly leaves its last coordination heartbeat
  fresh for up to 30 seconds. Once its own `broker stop` has stopped that
  incarnation and the registry proves its runtime stopped, resume expires the
  beat instead of refusing the relaunch as a live prior incarnation.
- An abrupt external kill can skip that broker cleanup. On macOS, absence of
  the process group alone cannot authorize replacing an unrevoked broker;
  resume may return `coordination-runtime-unverified` after tmux absence is
  confirmed. Follow [operator recovery after an external runtime stop](../specs/session-coordination-v1.md#operator-recovery-after-an-external-runtime-stop)
  to preview and apply `broker retire-stopped` for the exact incarnation, then
  resume the same session ID. Every retirement proof must pass, including stale
  heartbeat evidence; repeated resume does not bypass them. Prefer `restart`
  below when applying launcher changes to a live session.

## Restart onto the installed provider

`restart` relaunches a running session in place, for example after the
provider binary was upgraded on disk. The session id, provider conversation,
tmux runtime name, mailbox, and routing stay the same; the runtime gets a new
incarnation that runs whatever provider binary is installed now.

```bash
agent-session restart <id> --format json
agent-session restart <id> --force --stop-timeout 30 --ready-timeout 60
```

- Admission requires the same verified idle prompt and empty composer as a
  guarded idle send. A busy or unverified session returns
  `restart-session-not-idle` and is left running. An exact mailbox
  notification submission already in progress refuses the restart before
  anything is typed.
- Before the exit command, the restart fences the incarnation: new
  notification attempts and `send` input (`session-restarting`) are refused
  until the old runtime is stopped. A second restart of the session while
  one is in progress returns `restart-in-progress`.
- An admitted session gets the provider's own exit command (`/exit` for
  Claude Code, `/quit` for Codex). A provider still running after
  `--stop-timeout` seconds is left running with `restart-exit-timeout`; only
  `--force` terminates it through the verified stop path, and `--force` on an
  unverified session terminates without typing anything. `--force` never
  overrides identity: a pane that does not prove the session's runtime returns
  `restart-identity-unverified`, and an in-progress notification still refuses.
- A runtime another operation replaced during the exit wait is kept and
  reported as `restart-superseded`. A session record another writer holds for
  longer than `--stop-timeout` after the exit returns `restart-stop-failed`.
- The stopped incarnation is retired before the relaunch; a heartbeat that
  cannot be proven stale is waited out for a bounded time.
- The relaunch resumes the exact provider resume metadata, then waits up to
  `--ready-timeout` seconds, every observation included, for the provider to
  report an idle prompt.
- Success returns an `agent-session.restart.v1` receipt with the old and new
  incarnation, generation, pane pid, and provider version, plus timestamps
  and `stop_method` (`provider-exit` or `terminated`). The provider version is
  probed from the installed binary; an old version is known only for a runtime
  that a previous restart launched.
- Every failure after the exit command returns the receipt and a `recovery`
  command in the error details: `agent-session resume <id>` when the session
  is stopped (`restart-relaunch-failed`, `restart-prior-incarnation-live`, or a
  relaunched provider that exited), otherwise `agent-session glance <id>`
  (`restart-exit-timeout`, `restart-superseded`, `restart-stop-failed`,
  `restart-post-launch-failed`, `restart-not-ready`).
- A session cannot restart itself: the stop would end the command.

Retired orchestration registries and sidecars are ignored. Session records
retain unknown metadata through ordinary rewrites without projecting it or
restoring mode authority. Retirement does not migrate or delete durable state.

## Delete a session

By default, `delete` removes provider runtime files and session metadata only after bounded
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
`killed: true` field remains stable for ordinary successful deletion.
Its `termination_scope: "recorded-boundaries"` describes the verified recorded
runtime boundary; it does not certify that no escaped process exists.
Observed processes carrying both runtime markers block ordinary deletion and
retain the session record, with one narrow Linux exemption: a shared `sccache`
server whose executable basename is `sccache`, whose parent is PID 1, and whose
command line has no work arguments. The executable is checked through `/proc`,
not the process display name. Missing inspection data does not grant an exemption.
Attached cache processes, compiler wrappers, and all other survivors still block.
Recorded process-boundary verification remains required before this exemption is
considered; a daemon still inside that boundary does not bypass termination.

Retained servers are reported as `shared-daemon-retained: <pid> sccache` in text
and in the optional `process_inventory.retained_shared_daemons` array in JSON:

```json
{
  "retained_shared_daemons": [
    { "pid": 12345, "command": "sccache", "reason": "shared-daemon-retained" }
  ]
}
```

The same inventory field reports retained servers when another survivor blocks
deletion. Retaining a shared daemon never grants signaling authority over it.

Every deletion success includes `process_inventory` with `complete`,
`inspected_processes`, `uninspected_processes`, and a `reasons` count map.
An incomplete inventory remains advisory on top of recorded-boundary verification;
ordinary success prints the counts and reasons and warns that additional processes
may remain. For example:

```json
{
  "termination_scope": "recorded-boundaries",
  "process_inventory": {
    "complete": false,
    "inspected_processes": 12,
    "uninspected_processes": 2,
    "reasons": { "permission-denied": 2 }
  }
}
```

### Delete only an already-stopped session

Use `--if-stopped` when a caller must remove session state without asking
`agent-session` to stop the runtime. The caller must provide the expected
runtime incarnation:

```bash
agent-session delete <id> --if-stopped --expected-incarnation <incarnation> --format json
```

This option is mutually exclusive with `--leave-processes`. It removes the
record only after rechecking the incarnation under the lifecycle lock and
positively verifying that tmux and the recorded process boundaries are stopped,
with no associated processes remaining. It never signals the runtime. Success
reports `killed: false` and `termination_scope: "none"`.

The operation fails closed with `session-incarnation-conflict` if the record
has changed, `session-state-mismatch` if it is running or still has associated
processes, and `session-status-unknown` if stopped state cannot be proved. If
`session-termination-in-progress` is returned, a durable interrupted
termination still owns recovery (including any thaw obligation); retry with
ordinary `delete` to complete that recovery before removing the record.

### Leave processes running explicitly

A daemon can escape the pane tree or move out of its Linux cgroup while retaining
`AGENT_SESSION_ID` and `AGENT_SESSION_RUNTIME_ID`. A failed ordinary delete now
includes `blocking_processes` entries with `pid` and `command` (executable name
only). Boundary-verification failures report `retryable: false` when associated
live processes are observed, with an explicit recovery action.
Stop or relocate those processes deliberately, or acknowledge leaving them behind:

```bash
agent-session delete <id> --leave-processes --format json
```

This CLI-only recovery path checks ownership, conditionally kills the exact tmux
session, verifies its absence, revokes coordination capabilities, and removes
session state. It deliberately does not signal process groups, the provider,
broker processes, or escaped daemons directly. Processes that survive tmux shutdown
remain running, lose normal session management, and appear in `survivors`.
The response reports `termination_scope: "tmux-only"` and `killed: false`;
it does not claim that the provider or every session-launched process stopped.
Text output gives the same warning and names the observed survivors.

The process inventory is a bounded snapshot of same-user processes carrying both
exact runtime markers, on Linux and macOS. Arguments and environment values are
never printed. `survivor_scan_complete: false` (or `process_scan_complete: false`
in errors) means permissions, process churn, the time budget, or the 128-entry
limit prevented a complete inventory. Processes that removed their markers may
remain unlisted. Inspect survivors before taking any further action.
The same inventory counts and reasons accompany this explicit recovery result.
Reason classes include `permission-denied`, `process-read-failed`,
`argv-only-or-sip-redacted`, `process-changed`, `time-budget-exhausted`, and
`entry-limit-reached`, and `process-list-output-limit`. Counts cover observed
inspection attempts; a time, output or entry
limit can leave an unknown number of additional processes uninspected.

On macOS process enumeration has a separate 1 MiB output bound; exceeding it
reports incomplete inventory instead of silently accepting a process-list prefix.
The scanner uses the kernel's argument-size limit. System Integrity
Protection can omit a protected process's environment even when its arguments
were readable; an absent environment is reported as uninspected rather than
proof that the process has no runtime markers. No elevated privileges or SIP
changes are required. Ordinary deletion still verifies its recorded boundary;
the extra inventory cannot prove complete termination outside that boundary.

`--leave-processes` preserves the existing child-session guard; use
`--orphan-children` separately when deliberately orphaning managed child sessions.
Ownership mismatches and ambiguous tmux probes still retain the record. An
interrupted frozen/kill transaction must be recovered through ordinary deletion
before this flag can be used. Retired external runtimes remain unsupported.

## Startup failures

A fresh `start` or HTTP create that observes the provider stop before readiness
returns `start-failed` (CLI exit 1). Inspect `error.details.reason`,
`provider_exit_code`, and the optional redacted `stderr_tail` for the cause.
The retained `session_id` makes the failed attempt addressable. Retry with a
fresh session ID only when `retryable` is true; uncertain cleanup requires
inspection first.

`startup-trust-required` identifies a folder-trust screen. Review the requested
working directory and configure its provider trust entry before retrying.
`startup-prompt-required` identifies a recognized sign-in or setup screen.
The launcher does not accept these prompts automatically and attempts verified
cleanup so the failed create does not leave a silent live orphan.
An unobservable or still-starting runtime is not proof of failure; reconcile
that attempt before creating another session.

## Per-session temporary directory

Every managed tmux session launches with `TMPDIR=${AGENT_HOME}/tmp/<session-id>`
(mode 0700), so harness scratch directories and `mktemp` output stay under
`AGENT_HOME` instead of the system `/tmp`. This is the `tmp` class of the agent
write-location contract in agent-runtime-kit
([`write-locations.md`](https://github.com/sympoies/agent-runtime-kit/blob/main/core/policies/write-locations.md)).
`AGENT_HOME` defaults to `${XDG_STATE_HOME:-$HOME/.local/state}/agent-runtime-kit`.
Only the new tmux session sees `TMPDIR`; `TMUX_TMPDIR`, the launching process and
launchd services are unchanged.

- The directory is created at start, resume and restart, and `agent-session` records
  an owner file in it. A sweep only touches directories whose owner file names the
  same session and `--state-dir`.
- When the runtime exits, the grace period starts (default 24 hours, set with
  `AGENT_SESSION_TMP_GRACE_SECS`). Relaunching the session within the grace period
  keeps the contents; after it, the contents are removed before the relaunch.
- `agent-session tmp sweep [--grace-secs N] [--format json]` removes directories of
  stopped sessions past the grace period and of deleted session records, and reports
  what it removed and kept. A running or unprovable session, or one whose record is
  locked, is never removed. Every launch also sweeps best effort, within two seconds.
- To keep the system `TMPDIR` for one session, start it with
  `--env AGENT_RUNTIME_SESSION_TMPDIR=system` (a public launch switch, see below).

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
  to apply; re-selecting the current account cancels a queued switch. A
  binding shown as `failed` after a serve restart is repaired the same way,
  including by re-selecting its own account.
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

## Public launch switches

`agent-session start --env AGENT_RUNTIME_SUPPRESS_MEMORY=1` passes a switch to
both the provider process and its startup hooks. Repeat `--env KEY=VALUE` for
multiple switches. This is a public, non-secret surface: keys and values are
persisted as `launch_env` and returned by JSON start/list/board responses.
Never pass credentials or private values.

The default allowlist contains `AGENT_RUNTIME_SUPPRESS_MEMORY`,
`AGENT_RUNTIME_SUPPRESS_HEALTH`, `AGENT_RUNTIME_SUPPRESS_PREFLIGHT`,
`AGENT_RUNTIME_SUPPRESS_FINISH_GATE`, and `AGENT_RUNTIME_SESSION_TMPDIR`. Configure
`AGENT_SESSION_LAUNCH_ENV_ALLOWLIST` as a JSON array to replace that list;
`[]` disables overrides. Configured keys must be uppercase `AGENT_RUNTIME_`
switch names without credential words. At most 16 unique assignments are
accepted, with values bounded to 1024 bytes and no control characters.
Values are literal; empty values and `=` within values are preserved.

The Console start path forwards `launch_env` to the target daemon and requires
an exact echo. A relay that strips the field produces `launch-env-unconfirmed`
after creation; inspect the created session before retrying. Configure the
same allowlist on the caller and target daemon for custom switches. Resume,
account switching, clear/rebind, and runtime recreation retain the saved map;
a recreated provider receives the same overrides. Recreation revalidates the
current allowlist and refuses a saved key that is no longer approved. With no
overrides, existing launch environment inheritance is unchanged.

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
