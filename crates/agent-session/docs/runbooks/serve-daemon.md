# Serve Daemon Operations

This runbook covers safe startup and routine integration of
`agent-session serve`. The daemon exposes the local session control plane over
HTTP and WebSocket while reusing the same lifecycle implementation as the CLI.

## Start safely

Bind to loopback and pass the bearer token on stdin so it does not appear in
process arguments or the daemon environment. In this interactive example the
temporary shell variable must remain unexported:

```bash
read -r -s AGENT_SESSION_SERVE_TOKEN
printf '%s' "$AGENT_SESSION_SERVE_TOKEN" | \
  agent-session serve \
    --bind 127.0.0.1:8781 \
    --token-stdin
unset AGENT_SESSION_SERVE_TOKEN
```

The daemon refuses a non-loopback bind unless `--allow-non-loopback` is
explicitly passed. It controls a remote shell, so the recommended deployment is
an authenticated edge in front of the loopback daemon. Expose the edge, not the
raw serve port.

`--token-stdin` accepts one non-empty token of at most 8192 bytes and conflicts
with `--token`. The CLI still accepts `--token` and `AGENT_SESSION_TOKEN` for
compatibility, but do not use the environment form for a daemon that creates
managed sessions: the current child-launch path can inherit that variable and
thereby grant a provider the machine-operator bearer. Prefer a private
credential source connected to stdin. When no non-empty token is configured,
authenticated endpoints fail closed.

One serve owns a state root for its whole lifetime through an exclusive lock on
`<state-dir>/serve.lock`, taken before any recovery, account fencing, endpoint
publication, or background loop. A second serve on the same canonical root,
including through a symlinked spelling, waits briefly for a restarting
predecessor, then exits `69` with `serve-state-root-owned` and the holder's pid
and bind address. Give a development or test serve its own `--state-dir` or
`AGENT_SESSION_STATE_DIR`; separate roots run side by side.

## Authentication boundaries

| Endpoint class | Authentication and exposure |
| --- | --- |
| `GET /healthz` | Open on the bind address; health metadata only. |
| `GET /sessions` | Open; may include working directories and recent prompt previews. |
| `GET /usage` | Open; returns provider usage projections. |
| `GET /sessions/{id}/glance` | Open; returns recent live pane content. |
| `GET /sessions/{id}/buffer` | Open; returns the tmux server's latest global clipboard buffer after only checking that the session exists. |
| `GET /sessions/{id}/auto-resume` | Open; returns that session's auto-resume state. |
| Path-bearing reads, account inventory, activity stream, writes, and WebSocket attach | Bearer token. |
| Public coordination and broker reads | Bearer token. |
| Session board reads (`GET /board/v1`) | Bearer token, and only when started with `--board` or `AGENT_SESSION_BOARD=1`; otherwise `board-disabled` (HTTP 404). Returns home-relative working directories and the machine identity. |
| Session-owner coordination and mailbox mutations | Bearer token plus `X-Agent-Session-Capability`. |

Loopback prevents remote network access; it does not authenticate local
processes or users. Run the raw daemon only on a trusted single-user host or
behind a local access-control boundary that prevents untrusted same-host
principals from reaching the port. Any local caller that can connect can read
the open prompt, pane, path, usage, clipboard, and auto-resume projections.

The bearer token is machine-operator authority. The session capability is
per-incarnation owner authority. Session IDs and other request selectors do not
replace either credential.

Browser WebSocket clients cannot set an `Authorization` header. The edge must
proxy the attach and inject the bearer server-side; never put the token in a
WebSocket URL or query string.

## Prompt preview lifecycle

`GET /sessions` may return a running Codex or Claude session's `last_prompt`
only when the record carries an exact provider resume identity and the matching
regular transcript can be validated. On first discovery the daemon establishes
an append offset, queues an at-most-64-MiB cold recovery outside the list-response
path, then retains only the latest bounded preview in process memory. Recovery is
single-flight per session and daemon-wide concurrency-bounded. Later list
requests perform only a bounded freshness check; appended chunks are consumed in
the background instead of repeating the cold scan.

The preview cache is not daemon state: it is never written to the session
record, logs, or diagnostics, and a daemon restart reconstructs it from the
provider transcript. Eligible running sessions report `last_prompt_state`:

- `current` means the exact tracker is caught up. A missing `last_prompt` in
  this state authoritatively means no eligible user prompt exists.
- `pending` means exact discovery, cold recovery, or append catch-up is in
  progress. The preview stays omitted rather than exposing a stale cached
  prompt.
- `unavailable` means no exact source can currently be used or continuity was
  invalidated. The preview stays omitted and the old cached value cannot cross
  the API boundary. After invalidation it remains visible to every caller until
  one response can expose an authoritative `current` projection.

Eligible states also carry the opaque response-only
`last_prompt_continuity` token. It rotates when exact transcript continuity is
lost and on a daemon restart, so a consumer that missed an intermediate
`unavailable` response cannot re-display a preview from the prior source. The
token is 16-128 URL-safe ASCII characters and contains no prompt, path, provider
session id, or runtime identifier.

Transcript rotation, truncation, replacement, or identity drift clears the
cached value and requires exact rediscovery. Stable admission at the registry
bound prevents overflow sessions from repeatedly evicting warm recovery state.
A missing provider resume identity makes the session ineligible, so both state
and preview remain intentionally omitted rather than authorizing a likely
transcript scan.

Because `GET /sessions` is open on the loopback bind, treat prompt previews as
sensitive local-user data. Aggregate health checks should count preview presence
by provider without printing prompt text, session ids, resume ids, or transcript
paths.

Session-list coordination summaries read an observational registry snapshot.
They do not renew claims or operation leases, and filesystem-backed conflict
checks run after the registry lock is released. Registry maintenance remains on
the mutating coordination paths.

## Create a session

`POST /sessions` accepts JSON. For example:

```json
{
  "agent": "codex",
  "cwd": "/workspace/example",
  "title": "Review the API",
  "prompt": "Inspect the current change",
  "coordination_mode": "advisory",
  "agent_args": []
}
```

The required field is `agent`. Fresh creation may also use `cwd`, `title`,
`title_state`, `id`, `prompt`, `coordination_mode`, `agent_args`, an advertised
`agent_profile`, and—for supported fresh Codex sessions—`codex_account`.
`coordination_mode` accepts `advisory`, `enforce`, or `off` and defaults to
`advisory`.

Provider import uses `provider_resume_id` (compatibility alias: `resume_id`).
When `agent_profile` is omitted, discovery uses the daemon's default provider
history. When a profile is selected, it must advertise import support and
discovery is confined to that profile's provider root. In either import mode,
omit `cwd`, `prompt`, `agent_args`, and `codex_account`; the daemon resolves the
original working directory and exact provider metadata from the selected
history source. A capable Codex import uses the same daemon-managed app-server
runtime as a fresh Codex session, preserving account and auto-resume controls;
an unsupported or explicitly raw Codex runtime keeps the standalone resume
fallback.

The server owns executable paths, provider configuration roots, readiness
commands, and auto-resume capability. Clients select only advertised safe IDs
and cannot override those private fields.

## Use the control plane

The main endpoint groups are:

- session inventory, glance, usage, work-directory discovery, and activity
  stream;
- session create, title update, send, prompt, resume, account selection,
  auto-resume, attachment upload, and delete;
- WebSocket PTY attach;
- raw work-context, broker recovery, and mailbox operations.

Ordinary JSON HTTP responses use the `cli.agent-session.serve.v1` envelope.
Successful responses include the machine identity in `data.machine`, selected
by `--machine`, `AGENT_SESSION_MACHINE`, `--host`, or the hostname fallback;
current error envelopes omit it. The activity SSE stream and WebSocket attach
use their own streaming protocols rather than that JSON envelope.

Attachment upload is a raw binary stream. The daemon defaults to a 1 GiB
per-file ceiling and writes request chunks to a private temporary file before
syncing and atomically publishing the final attachment. Set
`AGENT_SESSION_MAX_ATTACHMENT_BYTES` before startup to an integer from 1 byte
through 16 GiB when a deployment needs a different bound. A rejected,
disconnected, or failed request never returns an attachment path and removes
its temporary file.

Use `POST /sessions/{id}/prompt/v2` when a client needs a cross-version fence.
It requires both exact prompt text and the expected session incarnation,
rejects unknown fields, and is absent from older daemons. A replaced runtime
returns `409 session-incarnation-conflict` before provider dispatch.

### Recover a locally busy Codex prompt

A resumed Codex TUI can remain visible while ordinary terminal submission is
rejected by the local app-server proxy with JSON-RPC code `-32001` and
`agent-session state is busy; retry the request`. Current proxies preserve that
stable code and message and add `error.data.reason`, a closed-vocabulary reason
that contains no prompt, account, path, thread, session, or credential values.
For example, `account_not_ready` means the proxy could not validate the durable
binding for its exact runtime; `turn_gate_busy` and `turn_already_pending` mean
serialization is still occupied; `manual_marker_*` reasons identify a stale or
mismatched Agent Console sender authority. Repeating Enter or replaying the same
terminal bytes does not repair that control state and can make prompt delivery
ambiguous.

A managed account that is already durably `bound` to the exact runtime remains
valid inside a detached tmux scope even though that scope intentionally does
not inherit the daemon's credential-broker command. The broker remains required
for binding and account mutation; it is not required to re-authorize each TUI
turn after the binding has been applied.

The busy response alone does not prove that the continuation was rejected
before provider delivery: the proxy can return the same response while an
earlier request is already pending. First refresh the session and inspect its
provider-visible activity/output. If a turn is in progress, the continuation
appears accepted, or non-delivery cannot be established, wait and observe; do
not send the continuation again.

Only after confirming that the continuation was not accepted and no provider
turn remains in progress, use the authenticated edge or another trusted
loopback client to read the session's current `session_incarnation`, then
submit the continuation through the fenced route exactly once:

```json
{
  "text": "Continue from the current task.",
  "expected_session_incarnation": "CURRENT_SESSION_INCARNATION"
}
```

Send that body to `POST /sessions/SESSION_ID/prompt/v2`. Success returns
`submitted: true` and the same incarnation. A `409
session-incarnation-conflict` means the runtime was replaced; refresh the
session list and decide against the new incarnation instead of removing the
fence. Any outcome-unknown transport failure requires observation before a
retry. The incarnation fence prevents submission to a replacement runtime; it
does not deduplicate two submissions within the same incarnation. This
procedure is for a provider-ready Codex control channel; it does not turn an
unsupported raw TUI into a structured-prompt target.

For work coordination, consult [Work coordination](work-coordination.md).
High-level self-targeting CLI operations such as `work-context set` do not have
HTTP convenience routes; the daemon exposes the raw v1 operations documented
in the [coordination route matrix](../specs/session-coordination-v1.md#http-coverage).

## Keep sessions alive across daemon restarts

By default, a child tmux server shares the caller's cgroup. Under systemd this
can allow a service restart to kill live sessions. Set
`AGENT_SESSION_TMUX_SCOPE=1` to request a transient systemd user scope for the
tmux server:

```bash
AGENT_SESSION_TMUX_SCOPE=1 agent-session serve ...
```

When a user systemd manager or `systemd-run` is unavailable, the daemon falls
back to direct tmux launch. Pair the isolated scope with `KillMode=process` on
the serve service for defense in depth.

### Restart after a binary upgrade

A package-manager upgrade replaces or removes the installed `agent-session`
while serve is still running, which would leave new session launches unable to
exec their helper. When `AGENT_SESSION_TMUX_SCOPE` is enabled, serve records the
device and inode of its own executable at startup, resolving a linked
invocation path such as a Homebrew `bin/` link to the release file, and checks
it once per second. When that file is replaced or removed, or the invocation
link is repointed at another release while the old one stays installed, serve
logs `serve-binary-replaced` to stderr, stops accepting connections, gives
in-flight HTTP requests and streams up to 10 seconds to finish, and exits `75`
(`EX_TEMPFAIL`). Serve itself kills no tmux session; the scoped panes survive
exactly as they do across a manual restart.

Without the tmux scope, sessions share the service cgroup, and a supervisor
cleaning up the exited service would kill them. Serve therefore does not watch
its binary unless the scope is enabled; an unscoped serve keeps running on the
replaced binary until it is restarted deliberately.

The supervisor must restart on that non-zero exit:

- systemd: `Restart=on-failure` (or `Restart=always`), together with
  `KillMode=process` and `AGENT_SESSION_TMUX_SCOPE=1` as above.
- launchd: `KeepAlive` set to `true`, or a `KeepAlive` dictionary with
  `SuccessfulExit` set to `false`.

A scoped deployment that restarts serve itself can opt out with
`AGENT_SESSION_SERVE_EXIT_ON_BINARY_CHANGE=0`; serve then keeps running on the
replaced binary.

At startup, historical session records may remain even when tmux has no server
or live sessions. A recognized tmux missing-server diagnostic is an
authoritative empty live-session snapshot, so the Codex account reconnect fence
does not block the HTTP listener. Executable failures, permission errors,
and unrecognized non-success diagnostics remain unavailable and keep the fence
fail-closed.

## Optional integrations

- `AGENT_SESSION_CODEX_ACCOUNT_BROKER`: JSON argv array for a bounded host
  credential broker. Credentials remain in memory and are not projected into
  session documents or HTTP responses.
- `AGENT_SESSION_LAUNCH_PROFILES`: JSON array of server-owned launch profiles.
  Only profiles whose executable, optional provider root, and readiness probe
  pass are advertised. A Hermes-backed DSH profile may add an absolute
  `dsh_history.command`, absolute `dsh_history.root`, and `zstd` or `none`
  compression. Adapter availability affects history reads only, never profile
  readiness.
- `AGENT_SESSION_CODEX_RUNTIME=raw|app-server`: force the Codex runtime choice.
  The default probes the installed CLI and degrades to raw TUI when the audited
  app-server capability is unavailable.
- `AGENT_SESSION_USAGE_TIMEOUT_MS`: bounds provider usage collection.

## Operational checks

1. Confirm `GET /healthz` succeeds on loopback.
2. Confirm unauthenticated writes and attach fail.
3. Confirm authenticated `GET /sessions` reports the expected machine and
   capability projection.
4. Create a disposable advisory session with JSON `coordination_mode`.
5. Verify list/glance, prompt or send, and delete through the edge.
6. If coordination is integrated, separately verify operator-only reads and
   bearer-plus-capability owner mutations.
7. Restart the daemon and confirm expected tmux survival before relying on the
   service configuration.

The normative endpoint surface is in [Serve API v1](../specs/serve-api-v1.md).
Wire-level activity, coordination, turn-state, and maintenance contracts are
indexed in the crate [documentation map](../README.md).

## Cross-host mailbox configuration

Set these daemon-private values together:

- `AGENT_SESSION_RELAY_URL`: the Console edge origin (HTTPS; loopback HTTP allowed for isolated fixtures).
- `AGENT_SESSION_RELAY_TOKEN`: this machine's dedicated outbound relay bearer.
- `AGENT_SESSION_RELAY_INGRESS_TOKEN`: this machine's dedicated inbound relay token.

Machine identity uses the existing `--machine`/`AGENT_SESSION_MACHINE` contract.
The existing operator bearer must be configured and distinct from both relay
credentials. Partial config or unsafe URL/token combinations fail startup.
The edge's `AGENT_CONSOLE_COORDINATION_RELAYS` maps each machine to its outbound
and ingress credentials. Provider launches remove these secrets (including the
edge aggregate) from inherited process and tmux environments.

Agents use their normal session capability and the same state root as their
daemon. `message peers --session "$AGENT_SESSION_ID" --format json` discovers
eligible destinations. Send with `--to-machine`, then poll `message delivery`;
`queued` is not a remote persistence receipt. Existing inbox/show/ack commands
operate locally, and reply automatically routes to the original source machine.

Installed-artifact isolated acceptance (Node builtins only):

```sh
node crates/agent-session/tests/fixtures/remote-mailbox-acceptance.mjs \
  --agent-session-bin /absolute/immutable/release/agent-session
```

Add `--previous-agent-session-bin` for an old-writer roundtrip: message mutation
preserves remote origin and receipt TTL, broker authentication stays available,
and an old reply cannot target a local session. The fixture creates private temporary roots, real HTTP
daemons, an authenticated relay fixture, synthetic capabilities and exact child
cleanup. It prints a content-free JSON result. It does not exercise real Console
ownership; edge tests and fleet acceptance cover that boundary.

Disabling all three federation values preserves local messaging and remote state.
Rollback may restore an older daemon for local operations: registry schemas and
existing sessions stay unchanged. Retain `coordination/federation-journal.json`;
pending remote deliveries pause until a federation-capable daemon resumes.
