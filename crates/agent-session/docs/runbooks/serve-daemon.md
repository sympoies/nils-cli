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
| Session board reads (`GET /board/v1`, `GET /board/closed/v1`) | Bearer token, and only when started with `--board` or `AGENT_SESSION_BOARD=1`; otherwise `board-disabled` (HTTP 404). Returns home-relative working directories and the machine identity. |
| Session board relay (`GET /sessions/{id}/board/v1`) | The current session capability as `Authorization: Bearer`, and only with the board enabled. Forwards the query to the federation aggregator with the relay token; `board-relay-disabled` (HTTP 409) without federation. |
| Session-owner coordination and mailbox mutations | Bearer token plus `X-Agent-Session-Capability`. |
| `POST /activity/hook/v1` provider hook ingress | `X-Agent-Session-Capability` only, from a direct loopback peer. |

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
`agent_profile`, and—for supported fresh Codex sessions—`codex_account`, or—for
profile-free fresh Claude sessions—`claude_account`.
`coordination_mode` accepts `advisory`, `enforce`, or `off` and defaults to
`advisory`.

Provider import uses `provider_resume_id` (compatibility alias: `resume_id`).
When `agent_profile` is omitted, discovery uses the daemon's default provider
history. When a profile is selected, it must advertise import support and
discovery is confined to that profile's provider root. In either import mode,
omit `cwd`, `prompt`, `agent_args`, `codex_account`, and `claude_account`; the daemon resolves the
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

### Report provider hooks without state-directory writes

A provider whose file sandbox makes the state directory read-only can still
report turn activity. Append `--via http` to each hook command, for example
`agent-session activity hook --agent dsh --event pre_llm_call --via http`.
The hook then posts the same payload to the daemon's loopback
`POST /activity/hook/v1`, authenticated by the session capability that the
managed runtime already exports through `AGENT_SESSION_CAPABILITY_FILE`.

The sandbox must still allow the hook to read that capability file and
`<state-dir>/coordination/daemon-endpoint.json`, and to connect to the
daemon's loopback port. The daemon must be running: a hook that cannot reach
it is dropped silently, whereas the default `--via file` transport works
without a daemon. To diagnose, compare `agent-session activity status <id>`
before and after a turn; ingestion failures still appear in
`activity doctor`, because the daemon records them. The full contract is in
[Activity stream v1](../specs/activity-stream-v1.md#provider-hook-ingress).

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

## Recover sessions after a host reboot

A host reboot ends tmux runtimes but retains session and provider History.
Recognized missing-server/socket diagnostics are stopped evidence in inventory,
maintenance and exact-target verification; permission failures, timeouts and
unrecognized output remain unknown.

New macOS runtimes persist the kernel boot UUID with their private identity.
A verified prior boot allows deletion or exact provider resume without probing
or signaling numeric tmux/PID IDs reused in the current boot. Resume rotates
incarnation and generation while preserving provider identity, cwd and managed
account selection. This evidence does not grant authority over a current-boot
runtime or bypass assignment, quarantine or maintenance fences.

Older records without boot evidence may still fail coordination recovery.
When the exact managed tmux target is absent and no safe runtime boundary can
be established, maintenance v2 advertises **Remove from Console only**. That
confirmed action stops nothing; provider History remains available for a new
managed resume. Do not manually rewrite boot evidence or kill a global tmux
server to clear a card.

Acceptance must exercise disposable sessions through deletion and resume, then
verify a successful provider continuation and retained history/account. A
visible TUI/input prompt or healthy daemon alone does not prove writability.

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

The scope passes tmux its arguments verbatim. On systemd 254 or newer the
daemon adds `--expand-environment=no`, because newer `systemd-run` otherwise
expands `${VAR}` and `$$` in scope command arguments.

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
- `AGENT_SESSION_CLAUDE_ACCOUNT_BROKER`: JSON argv array for a bounded host
  Claude account broker (`agent-session.account-broker.v2`). It materializes a
  per-account `CLAUDE_CONFIG_DIR` and returns only the directory path; no token
  crosses it. Bound sessions resume only while it stays configured. See the
  [Claude account broker](../specs/serve-api-v1.md#claude-account-broker).
- `AGENT_SESSION_LAUNCH_PROFILES`: JSON array of server-owned launch profiles.
  Only profiles whose executable, optional provider root, and readiness probe
  pass are advertised. A Hermes-backed DSH profile may add an absolute
  `dsh_history.command`, absolute `dsh_history.root`, and `zstd` or `none`
  compression. Adapter availability affects history reads only, never profile
  readiness.
- `AGENT_SESSION_CODEX_RUNTIME=raw|app-server`: force the Codex runtime choice.
  The default probes the installed CLI and degrades to raw TUI when the audited
  app-server capability or a private runtime directory is unavailable. See
  [Codex runtime directory](#codex-runtime-directory).
- `AGENT_SESSION_USAGE_TIMEOUT_MS`: bounds provider usage collection.

The broker, launch profiles, retitle provider, and extra `PATH` entries can
instead come from one [configuration file](#configuration-file).

## Codex runtime directory

The Codex app-server runtime listens on a private Unix socket. Serve places it
below an absolute `XDG_RUNTIME_DIR` when one is set. A launchd job on macOS, or
a system service, has no such variable, so serve derives a per-user root
instead:

1. `<state-dir>/run`, when the socket path fits the Unix socket budget. This
   is persistent and outside system temp-file cleanup.
2. Otherwise `/tmp/agent-session-<uid>`, a short fallback for a long state
   directory. System temp cleanup may prune idle regular files there, so a
   deployment with long-lived sessions should shorten the state directory or
   set a short private `XDG_RUNTIME_DIR` instead.

The derived root is created mode `0700` and must be owned by the serving user,
not a symlink, and not group- or world-accessible, which is the same check
applied to `XDG_RUNTIME_DIR`. A deployment no longer needs to create the
directory or export `AGENT_SESSION_CODEX_RUNTIME=app-server` by hand.

A downgrade to the raw tmux runtime in automatic mode is never silent:

- `GET /codex/accounts` readiness reports `supported: false` with a stable
  `reason_code`, such as `codex-app-server-runtime-dir-unsafe`.
- Each affected session's `startup.runtime_fallback` carries the allowlisted
  reason, as listed in [Serve API v1](../specs/serve-api-v1.md).

To recover, fix the reported directory's owner or mode (`chmod 700`), or point
`XDG_RUNTIME_DIR` at a short private directory, then recreate the affected
sessions. Each new session resolves the directory again, so serve itself does
not need a restart unless its environment changed.

## Configuration file

`agent-session serve --config <file>` reads one versioned document instead of
the JSON-in-environment values above. The extension selects the syntax:
`.toml` or `.json`, with identical structure. The file is at most 256 KiB.

```toml
schema_version = "agent-session.serve-config.v1"

[path]
append = ["/opt/tools/bin"]

[codex_account_broker]
argv = ["/absolute/path/to/broker"]

[retitle]
provider = "openai_compatible"
base_url = "http://127.0.0.1:1237/v1"
model = "example-model"
api_key_env = "EXAMPLE_API_KEY"

[retitle.context]
max_chars = 12000

[[launch_profiles]]
id = "dsh-tui"
label = "DSH TUI"
agent = "hermes"
agent_bin = "/absolute/path/to/dsh"
```

Each table uses the fields and bounds of the value it replaces:
`launch_profiles` entries are the objects of `AGENT_SESSION_LAUNCH_PROFILES`,
`retitle` is the object described in
[Session retitle v2](../specs/session-retitle-v2.md#provider-configuration),
and `codex_account_broker.argv` is the argv array of
`AGENT_SESSION_CODEX_ACCOUNT_BROKER`. `path.append` holds at most 16 absolute
directories without `:` or control characters. Every table is optional; only
`schema_version` is required. An unknown key is an error rather than ignored.

Validate a document, together with the environment it will merge with, without
starting the daemon:

```bash
agent-session serve --config serve.toml --check --format json
```

`--check` exits before token resolution, state-root ownership, or any network
bind. On success it prints a `cli.agent-session.serve-config.v1` envelope that
names each input's source (`none`, `file`, `environment`, or `merged`), the
effective launch-profile ids, the number of configured `path.append` entries
(`data.path.append`, counted before inherited duplicates are skipped), and a
warning for every file value the environment overrides or shadows. It never
prints paths or values. The launch-profile readiness probe still runs only when
serve starts.

### Precedence

A non-empty environment variable takes precedence over the file. An empty or
whitespace-only variable counts as unset, as it does without a config file.

| Environment variable | Config key | When both are set |
| --- | --- | --- |
| `AGENT_SESSION_LAUNCH_PROFILES` | `[[launch_profiles]]` | Merged: environment entries first, then file entries in order. The first entry for an id wins; a later file entry with the same id is dropped with a warning. The merged list must stay within 16 profiles. |
| `AGENT_SESSION_RETITLE_CONFIG` | `[retitle]` | The environment value replaces the whole table, with a warning. |
| `AGENT_SESSION_CODEX_ACCOUNT_BROKER` | `[codex_account_broker] argv` | The environment value replaces the whole table, with a warning. |
| `PATH` (a launcher's appended entries) | `[path] append` | File entries are appended after the inherited `PATH`, never before it; entries already present are skipped. |

The ordered, first-id-wins launch-profile merge is the same one a launcher
performs when it concatenates several profile sources into
`AGENT_SESSION_LAUNCH_PROFILES`. File entries are validated strictly: duplicate
ids inside the file are an error rather than a dropped entry. An unavailable
profile executable is still not an error; the readiness probe simply does not
advertise that profile.

Serve applies the resolved values to its own environment before it starts any
thread, exactly as a launcher that exported them would. Managed sessions
therefore inherit the same `PATH` and variables they inherit today.

### Secrets

The schema has no field that holds a secret value. Credentials are referenced
by environment variable name, as in `retitle.api_key_env`. A credential-shaped
key is refused anywhere in the document, including inside `retitle.extra_body`.
Such a key contains `secret`, `password`, `passwd`, `apikey`, `credential`,
`bearer`, or `authorization`, or a pair such as `api_key`, `private_key`, or
`secret_key`, or ends in `token`. A key such as `stop_token_ids` only mentions
tokens and is accepted. A schema field such as `api_key_env` is a
reference and is accepted, but inside `extra_body` every key is sent to the
provider verbatim, so a `*_env` or `*_file` suffix does not excuse a credential
there. Supply an `extra_body` parameter whose name is credential-shaped through
`AGENT_SESSION_RETITLE_CONFIG` instead. Keep the serve bearer on
`--token-stdin` as described in [Start safely](#start-safely).

### Errors

A rejected document exits `64` before serving. With `--format json` the failure
is a `cli.agent-session.serve-config.v1` envelope; in text form it is one
`error: <code>: <message>` line on stderr. Messages and `error.details` name
the offending key (for example `launch_profiles[1].id` or `path.append[0]`) and
its `source` (`file` or `environment`), never the file location or a value.

| Code | Meaning |
| --- | --- |
| `serve-config-unreadable` | The file is missing, unreadable, or not a regular file (`details.reason`). |
| `serve-config-unsupported-format` | The extension is not `.toml` or `.json`. |
| `serve-config-too-large` | The file exceeds 256 KiB. |
| `serve-config-parse-failed` | Invalid TOML, JSON, or UTF-8; `details` carries only the line and column. |
| `serve-config-unsupported-version` | `schema_version` is missing or not `agent-session.serve-config.v1`. |
| `serve-config-unknown-key` | A key the schema does not define. |
| `serve-config-invalid-value` | A value outside the field's type or bounds, including an invalid `AGENT_SESSION_LAUNCH_PROFILES` it must merge with. |
| `serve-config-inline-secret` | A credential-shaped key holds an inline value. |

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

## Emergency shell prerequisites

Install `zsh` and `tmux` on hosts that expose emergency shells. Shell routes
require the machine bearer, including status reads. The edge owns principal and
machine authorization. Each principal keeps one fixed tmux session under the
existing host account; sharing that account means sharing host permissions.
Leaving a client detaches; an explicit fenced DELETE or `exit` stops the shell.
See [the Shell contract](../specs/serve-api-v1.md#emergency-shells) for the wire
protocol and fixed-name collision behavior. Daemon restarts preserve tmux but
host/tmux restarts require a new explicit Open.
