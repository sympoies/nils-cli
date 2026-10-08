# agent-session

Audit one configured state root without reading message bodies:

```sh
agent-session message audit --older-than 300 --format json
```

The [mail audit contract](docs/specs/mail-audit-v1.md) documents pagination,
operator authentication, anomaly codes and source/destination metadata joins.

## Overview

`agent-session` starts and manages tmux-backed Codex, Claude Code, and DSH sessions for mobile handoff workflows. It is designed for
personal automation such as the agent-console mobile control plane: a service can create the session with a full
prompt, then return a short tmux attach command for the user to continue from Termius, glance at the pane, or steer it with keystrokes.

## Package vs binary name

| Field        | Value                |
| ------------ | -------------------- |
| Package name | `nils-agent-session` |
| Binary name  | `agent-session`      |

## Documentation map

- Start here for positioning, common commands, and links: this README.
- Operate collision awareness and work permissions:
  [Work coordination](docs/runbooks/work-coordination.md).
- Deploy the HTTP/WebSocket control plane:
  [Serve daemon operations](docs/runbooks/serve-daemon.md).
- Integrate stable schemas and state machines:
  [Serve API v1](docs/specs/serve-api-v1.md),
  [Session Retitle v2](docs/specs/session-retitle-v2.md),
  [Session Retitle v3](docs/specs/session-retitle-v3.md),
  [Session public metadata v1](docs/specs/session-public-metadata-v1.md),
  [Session lineage and work v1](docs/specs/session-lineage-work-v1.md),
  [Session coordination v1](docs/specs/session-coordination-v1.md),
  [Session board v1](docs/specs/session-board-v1.md),
  [turn-state contract](docs/turn-state-contract.md), and
  [activity stream v1](docs/specs/activity-stream-v1.md).
- Browse every crate-local document by purpose:
  [agent-session documentation](docs/README.md).

## Usage

```bash
agent-session start --agent codex --cwd ~/Project/foo --prompt-file prompt.md
agent-session start --agent claude --issue sympoies/nils-cli#2032   # child of this session; inherits its program
agent-session work set <id> --issue sympoies/nils-cli#2040 --if-revision 1
agent-session lineage adopt <child> --by <steward>   # successor takes over a child
agent-session list
agent-session board --state live --since 3d --format json   # who else is working (session board v1)
agent-session glance <id> --tail 40
agent-session send <id> --text yes --key enter
agent-session send <id> --key c-c
agent-session send <id> --key down --key enter   # answer a dialog while blocked
agent-session send <id> --text "custom answer" --allow-blocked
agent-session resume <id>
agent-session account show <id> --format json
agent-session account switch <id> --account <nickname> --format json
agent-session activity status <id> --format json
agent-session activity doctor --format json
agent-session activity setup --agent codex --dry-run
agent-session activity setup --agent codex --repair --dry-run
agent-session activity setup --agent codex --repair --expected-preview-digest sha256:<reviewed-plan-digest>
agent-session metadata attach <id> --request-file metadata.json --if-revision 0 --idempotency-key attach-001 --format json
agent-session metadata show <id> --label acceptance.synthetic --format json
agent-session work-context status --format json
agent-session work-context set --tier issue --issue 123 --summary "Implement the tracked fix"
agent-session work-context advise --format json
agent-session work-context acknowledge --for 30m
agent-session work-context clear
agent-session message inbox --session <id> --category progress --category handoff
agent-session message forward --session <id> --message <message-id> --if-revision 1 --to <destination> --category progress --idempotency-key forward-001
agent-session serve --bind 127.0.0.1:8781 --token-stdin
agent-session command <id>
agent-session attach <id>
agent-session logs <id>
agent-session readiness --format json
agent-session delete <id>
agent-session completion zsh
```

`agent-session readiness --format json` authenticates the current managed runtime
and verifies its exact runtime-issued checkpoint file. It does not acquire a claim
or authorize mutations. Ordinary claims and admission remain separate.

Retired orchestration registries and sidecars are ignored. Session records retain
unknown metadata through ordinary rewrites without projecting it or restoring
mode authority. Retirement does not migrate or delete durable state.

`metadata attach` is a bounded state-owner primitive, not an approval system or
an arbitrary session-record editor. It accepts one owner-private
`agent-session.metadata-attachment.request.v1` file, compares the caller's
expected public-metadata revision, and binds an idempotency-key digest to the
validated label/value payload. `metadata show` returns only that public
projection; raw metadata values, prompts, logs, provider credentials, runtime
identities, request paths, and raw idempotency keys are excluded. See the
[Session public metadata v1 contract](docs/specs/session-public-metadata-v1.md)
for the exact limits, replay semantics, and stable error codes.

`send` pushes input to a live session: literal text (`--text` / `--text-stdin`) and/or repeatable named keys
(`--key enter|escape|backspace|c-c|up|down|left|shift-left|right|tab`), so codex/claude approval prompts and terminal editing
remain usable from a phone. Text is pasted with bracketed-paste markers (`tmux paste-buffer -p`), so a multi-line message
stays one prompt with its line breaks. `--text`/`--text-stdin` followed by exactly `--key enter` is a prompt submission:
`send` reads the pane back and reports `data.submission = { outcome, enter_presses }`. `submitted` means the text left the
Claude Code or Codex composer, and `queued` means Claude Code queued it behind a running turn ("Press up to edit queued
messages"). When the text is still in the composer, `send` presses Enter once more, unless the
session is `needs_input` or the pane shows a dialog. If the text is still there after that, `send` fails with
`send-submit-stuck` (`error.details.outcome = "stuck"`) and leaves the text in place. The text has already reached the
pane, so do not resend it: `glance` the session or clear the composer first. `unverified` means the pane could not
prove either way: another provider, an unrecognized layout, a dialog, or a first line that looks like a numbered
choice (`1. …`). Text without `--key enter`, or with other keys, only
types; `--key` alone presses keys as before.
`glance` returns the recent pane tail plus live status as a JSON contract for dashboard tiles (cheaper than a full attach).
`resume` recreates a missing tmux runtime only when the session has exact provider resume metadata; it never resumes the
latest provider conversation implicitly. Runtime metadata is persisted before launch so hooks see the new generation,
and the immutable tmux session/pane identity is persisted before a successful start or resume returns. Resume first
proves the current and every retained prior launch identity stopped, so a surviving provider process cannot be hidden by
a new runtime generation.
An older stopped record without that proof returns the same non-retryable manual-verification action as deletion; only
a generation durably marked as never launched can resume without a runtime identity. If tmux launch fails, the prior
runtime and activity snapshot are restored only after any possibly launched replacement is verified stopped. An
unverified replacement remains the current discoverable generation. `send` bumps `updated_at`, so `list` orders by real
control-plane activity.
`delete` removes provider runtime files and session metadata only after bounded checks verify the recorded runtime is
stopped. Before killing a live runtime, it inspects only the managed `0.0` pane, captures its immutable tmux session/pane
ids and process boundary, validates the runtime's `AGENT_SESSION_*` ownership markers, persists that identity for retry, and
uses one tmux-server conditional command to kill only if the captured session, pane, and pane PID still match.
If the managed pane is replaced, it durably retains every unresolved observed process identity before proceeding. It then
targets only the captured tmux session id. On Linux it snapshots each verified process-session member by PID and start time
and requires the pane to be in a leaf cgroup-v2 `tmux-spawn-*.scope`. It pins the process sets observed before and during a
bounded stabilization window after freezing that cgroup, revalidates the pane membership and cgroup inode, conditionally
kills the exact tmux identity, and
invokes `cgroup.kill` while the boundary remains frozen. Members present at either verified boundary cannot survive by
changing process session or by later cgroup migration. The cgroup identity also records the Linux boot id, so startup
recovery never applies an old PID/start-time or cgroup identity to a different boot. A durable ownership marker lets a
delete retry or a restarted server thaw only a scope that deletion changed from unfrozen to frozen; a scope that was already
frozen remains frozen. Without that distinct leaf cgroup, Linux deletion fails closed
before mutating tmux; other Unix platforms retain verify-only handling for the pane process-group boundary. Cleanup requires
tmux and every retained process boundary to be gone. A retry can
finish cleanup without another kill only when all persisted identities verify stopped. Live records created by an older
version can be upgraded from their ownership markers. A stopped pre-upgrade record without a provable launch identity remains
retained and returns `runtime-identity-unavailable`, `retryable: false`, and
`action: manual-runtime-verification-required`; an operator must verify its runtime manually before removing that state.
Kill failures, ambiguous tmux errors, ownership mismatches, and surviving processes retain all session state. Human
success output reports the verified stopped state; the v1 JSON `killed: true` field remains stable for successful deletion.
When tmux returns a successful but blank identity probe for a stale session, deletion confirms the exact session is absent
and still requires every persisted process boundary to verify stopped before removing metadata.
For a stopped session without a one-shot run log, `agent-session logs <id>` falls back to the private, tail-capped startup
diagnostic. Codex sessions retain that diagnostic after a startup failure or non-zero provider-client exit; a clean exit
after readiness discards it.
DSH panes start only through a server-owned `serve` launch profile with base agent `dsh`, which launches the profile's
`agent_bin` with no implicit subcommand; a profile-less `start --agent dsh` is refused, and one-shot `run` mode is
codex/claude only.

## Work coordination

Work coordination is advisory by default. Managed sessions publish
privacy-safe presence and can declare repository-relative exact paths or
prefixes; overlapping work warns without blocking unless the session starts
with `--coordination-mode enforce`. Session IDs and claim IDs are selectors,
not credentials. Owner operations use the private per-incarnation capability.

Coordination does not grant or revoke user authorization, repository
permission, provider consent, or workflow authority. In default `advisory`
mode, missing context, unavailable coordination, and overlap reports remain
non-blocking. `work-context set` adds optional public task metadata so warnings
are more precise; it is not a permission request. Only a launch that explicitly
selects `--coordination-mode enforce` turns claims, admission, and physical
checkout leases into mutation requirements.

Use [Work coordination](docs/runbooks/work-coordination.md) for the operator
workflow and path syntax. The normative schemas, state machines, authorization
rules, limits, error codes, and HTTP coverage live in
[Session coordination v1](docs/specs/session-coordination-v1.md).

The canonical agent-facing policy, including how an agent responds to overlap
advice, lives in agent-runtime-kit's
[`session-coordination.md`](https://github.com/sympoies/agent-runtime-kit/blob/main/core/policies/session-coordination.md).
This README defines CLI and operator semantics only.

## Turn-state integration

Supported provider hooks project metadata-only lifecycle events into a private,
runtime-bound activity snapshot. Provider registration is owned by
`agent-hook`; the retained `agent-session activity setup` command is a
compatibility forwarder:

```bash
agent-session activity setup --agent codex --dry-run
agent-session activity setup --agent codex --repair --expected-preview-digest sha256:<reviewed-plan-digest>
agent-session activity setup --agent codex --remove
agent-session activity doctor --agent codex --format json
```

The forwarder maps compatibility flags and validates the typed `agent-hook`
response. If the matching `agent-hook` binary is absent, setup returns
`agent-hook-setup-unavailable` without writing provider configuration. Existing
`activity hook`, `activity notify`, and read-only `activity doctor` paths remain
for runtime compatibility and migration diagnostics.

`activity doctor` remains read-only. Codex launch readiness requires a bounded,
strictly typed `agent-hook doctor` result for Codex whose status is
`converged`, whose owned count is exact, and whose retired residue is zero.
Missing, failed, malformed, oversized, multi-record, or mismatched
control-plane evidence fails closed. The diagnostic still recognizes exact pre-dispatch
`agent-session` registrations, including an audited Computer Use outer wrapper
at the fixed executable path under the active Codex config root, so an operator
can diagnose and migrate older installations. Existing `activity hook` and
`activity notify` ingestion paths
also remain as fail-open runtime compatibility while `agent-hook` becomes the
single provider-registration owner. `activity hook --via http` reports the same
payload to the daemon's loopback ingress with the session capability, for a
provider whose sandbox cannot write the state directory; see
[Activity stream v1](docs/specs/activity-stream-v1.md#provider-hook-ingress).

Turn state also gates input. While a session is `needs_input` its pane belongs
to a provider approval or question dialog, not to a prompt box, so the routes
that write to the pane refuse input carrying literal text with `agent-blocked`
before anything reaches the terminal. Special keys stay admitted — that is how
the dialog gets answered — and `send --allow-blocked` is the deliberate opt-in
for typing into the dialog's own field. Routes that do not touch the pane, such
as a Codex app-server prompt, are unaffected. It is a safety default rather than
an authorization boundary: the attach socket remains an interactive terminal.
See the [serve API contract](docs/specs/serve-api-v1.md) for the per-route table
and the exact codes.

Use the [turn-state contract](docs/turn-state-contract.md) for persistence,
privacy, registration ownership, setup, repair, migration, and provider
behavior. The evidence behind supported provider signals is recorded in
[provider turn-signal evidence](docs/provider-turn-signal-evidence.md).

## Serve daemon

`agent-session serve` exposes the local session control plane over HTTP and
WebSocket for an authenticated per-machine edge. Bind it to loopback, provide
the bearer token through stdin, and expose the edge rather than the raw port:

```bash
read -r -s AGENT_SESSION_SERVE_TOKEN
printf '%s' "$AGENT_SESSION_SERVE_TOKEN" | \
  agent-session serve --bind 127.0.0.1:8781 --token-stdin
unset AGENT_SESSION_SERVE_TOKEN
```

Keep that temporary shell variable unexported. The accepted
`AGENT_SESSION_TOKEN` compatibility input can be inherited by managed child
sessions, so it is not a safe credential source for a daemon that creates them.

Use [Serve daemon operations](docs/runbooks/serve-daemon.md) for deployment,
authentication boundaries, session creation, and restart survival. Integrators
should use the normative [Serve API v1](docs/specs/serve-api-v1.md), plus the
[activity stream](docs/specs/activity-stream-v1.md) and
[coordination](docs/specs/session-coordination-v1.md) contracts.

With `AGENT_SESSION_CLAUDE_ACCOUNT_BROKER` configured, each daemon-created
Claude session binds to a chosen account: the broker materializes a
per-account `CLAUDE_CONFIG_DIR`, the binding survives resume, and a switch
relaunches the session with `--resume` in the new account directory. See the
[Claude account broker](docs/specs/serve-api-v1.md#claude-account-broker).

`account switch <id> --account <nickname>` is the local, owner-run CLI for the
same switch as serve's `PUT /sessions/{id}/account`; both call one shared code
path and need no serve token. Claude queues the next account; when the session
is idle it preflights that account, stops the session and resumes the same
conversation under it, and while busy the switch stays queued. Codex binds the
account for the next prompt: the CLI durably queues it and the serve daemon's
control connection applies it at the idle boundary, so the next prompt is
fenced until then. `--expected-incarnation` fences the switch to one runtime
and defaults to the current one. JSON output returns the provider's
`codex_account` or `claude_account` view with `session_incarnation`, and
failures keep serve's typed codes (`claude-account-switch-refused`,
`<provider>-account-session-incarnation-conflict`,
`<provider>-account-unknown`, `<provider>-account-unsupported`).
`account show <id>` returns the current account and any queued `next` one.
Both commands, and `resume`, use the provider's account broker variable from
the caller's environment, or else the brokers serve recorded in the private
state dir at startup; with neither, `show` reports the account as unsupported.
Run from inside the session's own tmux session, a Claude switch only queues.
After stopping, the switch retires the stopped runtime's coordination
incarnation before resuming. If the resume still fails, it returns
`claude-account-switch-resume-failed` with the `agent-session resume <id>`
recovery command; the session is stopped with the account queued. A Codex
switch needs a running serve daemon to apply; re-selecting the current account
cancels a queued switch.

## Output contract

Human-readable text is the default. JSON is opt-in with `--format json` on command subcommands.

JSON output uses the workspace envelope: `schema_version`, `ok`, `data`, optional `warnings`, and `error` on failure.

## Secret-safety boundary

Prompts are stored under the local agent-session state directory and are not printed in command output. For sensitive prompts, prefer
interactive `start`; one-shot `run` may need to pass the prompt through the underlying agent process command line depending on that agent's
CLI capabilities. `send` routes literal text through a private (0600) buffer file loaded into tmux, so it never appears in the tmux
command line or command output; the JSON contract reports only `sent_text` (a boolean) and the special-key names, never the text itself.
Values passed with `--agent-arg` are persisted in the private session record so durable resume can recreate the same provider invocation.
Do not put secrets in provider arguments. For Claude sessions, provider identity/resume flags such as `--session-id`, `--resume`/`-r`,
`--continue`/`-c`, `--fork-session`, and `--from-pr` are reserved for agent-session so the stored resume identity stays exact.
For secrets, prefer `--text-stdin`: `--text <value>` still places the literal in agent-session's own process arguments (visible in `ps`
to same-user processes), exactly as the existing `--prompt` flag does. `send` is not idempotent — keystrokes are delivered before the
command returns, so a retry after a mid-delivery failure can re-send; callers that auto-retry should account for this.

## Session model settings

Session creation retains `model_settings` in the session record, scoped to the
runtime launch ID. `list --format json`, HTTP session responses, and board
records expose nullable `model` and `reasoning_effort` fields. Missing values
are unknown, including old records without explicit launch settings; consumers
must not infer values from accounts, profiles, or the title-generation model.

The launch projection recognizes Claude `--model` / `--effort` and Codex
`-m` / `--model`, `-c` / `--config` model and `model_reasoning_effort` overrides,
including local Codex `--oss` models. A local Claude-compatible launcher can
supply the same explicit flags. Provider config files, wrapper-internal settings,
and implicit defaults are unknown until the provider reports them.

Matching provider hooks can update bounded model/effort metadata. Claude
`SessionStart` model and structured `effort.level` metadata are projected by
`agent-hook` activity rules without forwarding prompts, paths, or raw provider
identities. Matching tool/stop hooks update effort and read at most 64 KiB from
the trusted primary transcript for the actual model. A model switch becomes
visible after the provider emits that evidence. Both helpers must be updated;
older helpers safely leave unavailable fields unknown.
Managed Codex app-server thread responses report resolved defaults, and successful
`turn/start` responses confirm later explicit model/effort changes. Observations
must match the active runtime and provider session; auxiliary sessions cannot
replace primary settings. A changed model without an effort observation clears
the previous effort. Labels resembling credentials or paths are omitted.
