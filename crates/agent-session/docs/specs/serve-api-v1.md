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
| `GET /healthz` | Open | [Observation](serve-observation-v1.md) |
| `GET /board/v1` | Bearer; only with `serve --board` or `AGENT_SESSION_BOARD=1` | [Session board v1](session-board-v1.md#daemon-local-snapshot) |
| `GET /board/closed/v1` | Bearer; only with `serve --board` or `AGENT_SESSION_BOARD=1` | [Session board v1](session-board-v1.md#closed-session-ledger) |
| `GET /board/programs/v1` | Bearer; only with `serve --board` or `AGENT_SESSION_BOARD=1` | [Session board v1](session-board-v1.md#programs) |
| `GET /shells/{owner}` | Bearer | [Emergency shells](serve-api-v1.md#emergency-shells) |
| `POST /shells/{owner}` | Bearer | [Emergency shells](serve-api-v1.md#emergency-shells) |
| `DELETE /shells/{owner}` | Bearer + incarnation body | [Emergency shells](serve-api-v1.md#emergency-shells) |
| `GET /shells/{owner}/attach?incarnation=...` | Bearer + incarnation query | [Emergency shells](serve-api-v1.md#emergency-shells) |
| `GET /sessions` | Open | [Observation](serve-observation-v1.md); [context usage v1](session-context-usage-v1.md) |
| `POST /sessions` | Bearer | [Session creation](serve-session-create-v1.md) |
| `GET /history/sessions` | Bearer | [History and archive](serve-history-v1.md) |
| `GET /history/sessions/{history_id}/messages` | Bearer | [History and archive](serve-history-v1.md) |
| `GET /history/sessions/{history_id}/images/{image_id}` | Bearer | [History images v1](history-images-v1.md) |
| `POST /history/sessions/{history_id}/star` | Bearer | [History and archive](serve-history-v1.md) |
| `POST /history/sessions/{history_id}/resume` | Bearer | [History and archive](serve-history-v1.md) |
| `GET /codex/accounts` | Bearer | [Codex account broker](serve-accounts-v1.md#codex-account-broker) |
| `GET /claude/accounts` | Bearer | [Claude account broker](serve-accounts-v1.md#claude-account-broker) |
| `POST /clipboard/unwrap/v1` | Bearer | This specification |
| `GET /activity/events` | Bearer | [Activity stream v1](activity-stream-v1.md) |
| `POST /activity/hook/v1` | Session capability only; direct loopback peers only | [Activity stream v1](activity-stream-v1.md#provider-hook-ingress) |
| `GET /usage` | Open | [Observation](serve-observation-v1.md) |
| `GET /usage/v1` | Bearer | [Provider usage and resets](serve-usage-v1.md#provider-usage-and-resets) |
| `POST /codex/reset/v1` | Bearer; account allowlist and idempotency key | [Provider usage and resets](serve-usage-v1.md#provider-usage-and-resets) |
| `POST /claude/reset/v1` | Bearer; account allowlist and idempotency key | [Provider usage and resets](serve-usage-v1.md#provider-usage-and-resets) |
| `GET /workdirs` | Bearer | [Observation](serve-observation-v1.md) |
| `GET /repos/remote-url` | Bearer | [Observation](serve-observation-v1.md) |
| `GET /sessions/{id}/glance` | Open | [Observation](serve-observation-v1.md) |
| `GET /sessions/{id}/work-context/v1` | Bearer | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `POST /sessions/{id}/work-context/check/v1` and `POST /coordination/work-context/check/v1` | Bearer; session capability optional where supported | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `POST /sessions/{id}/work-context/{claim,renew,release,admit,complete,reconcile}/v1` | Bearer + capability | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `GET /sessions/{id}/broker/v1` | Bearer | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `POST /sessions/{id}/broker/{adopt,reconcile}/v2` | Bearer + capability; recovery proof is in the request body | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `POST /sessions/{id}/broker/{adopt,reconcile}/v1` | Bearer + capability; retained transition alias for the v2 authorization contract | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `POST /sessions/{id}/operations/{lease_id}/operator-reconcile/v1` | Bearer; explicit confirmed operator attestation | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `POST /sessions/{id}/activity/provider-turn/operator-reconcile/v1` | Bearer only; explicit confirmed inactive attestation | [Provider-turn reconciliation](coordination-schemas-v1.md#operator-provider-turn-reconciliation) |
| `GET /coordination/messages/audit/v1` | Operator bearer | [Mail audit v1](mail-audit-v1.md) |
| `GET and POST /sessions/{id}/messages/v1` | Bearer + capability | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `GET /sessions/{id}/messages/{message_id}/v1` | Bearer + capability | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `POST /sessions/{id}/messages/{message_id}/{ack,reply}/v1` | Bearer + capability | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `GET /sessions/{id}/messages/{message_id}/wait/v1` | Bearer + capability | [Coordination HTTP coverage](coordination-interface-v1.md#http-coverage) |
| `GET /sessions/{id}/buffer` | Open | [Observation](serve-observation-v1.md) |
| `POST /sessions/{id}/{send,prompt,prompt/v2,resume}` | Bearer | [Prompt submission](serve-prompt-v1.md) |
| `GET /sessions/{id}/prompt/v2/receipts/{request_id}` | Bearer | [Prompt submission](serve-prompt-v1.md) |
| `POST /sessions/{id}/archive` | Bearer | [History and archive](serve-history-v1.md) |
| `GET /sessions/{id}/maintenance` and `POST /sessions/{id}/maintenance/actions` | Bearer | [Session maintenance v1](session-maintenance-v1.md#authentication-and-endpoints), successor [v2](session-maintenance-v2.md#negotiation) |
| `PUT /sessions/{id}/account` | Bearer | [Account switching](serve-accounts-v1.md) |
| `GET /sessions/{id}/auto-resume` | Open | [Mailbox and automation](serve-mailbox-automation-v1.md) |
| `PUT and DELETE /sessions/{id}/auto-resume` | Bearer | [Mailbox and automation](serve-mailbox-automation-v1.md) |
| `POST /sessions/{id}/attachments` | Bearer | [Terminal transport](serve-terminal-v1.md) |
| `GET /sessions/{id}/attach` | Bearer | [Terminal transport](serve-terminal-v1.md) |
| `PATCH and DELETE /sessions/{id}` | Bearer | [Title updates](serve-session-create-v1.md); [Deletion](serve-history-v1.md) |

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

See [Provider session history](serve-history-v1.md#provider-session-history).

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
[Session Coordination V1](coordination-schemas-v1.md#operator-provider-turn-reconciliation).

## Endpoint detail map

| Task | Canonical contract |
| --- | --- |
| Serve history and archive v1 | [Serve history and archive v1](serve-history-v1.md) |
| Serve observation v1 | [Serve observation v1](serve-observation-v1.md) |
| Serve mailbox and automation v1 | [Serve mailbox and automation v1](serve-mailbox-automation-v1.md) |
| Serve prompt submission v1 | [Serve prompt submission v1](serve-prompt-v1.md) |
| Serve account brokers and switching v1 | [Serve account brokers and switching v1](serve-accounts-v1.md) |
| Serve session creation v1 | [Serve session creation v1](serve-session-create-v1.md) |
| Serve terminal transport v1 | [Serve terminal transport v1](serve-terminal-v1.md) |
| Serve provider usage and resets v1 | [Serve provider usage and resets v1](serve-usage-v1.md) |

### Provider usage and resets

See [Provider usage and resets](serve-usage-v1.md#provider-usage-and-resets).

## Response and authentication

Ordinary JSON HTTP responses use the `cli.agent-session.serve.v1` envelope. Successful responses carry a
`machine` identity in `data.machine` (`--machine` / `AGENT_SESSION_MACHINE` / `--host` / hostname) so an
edge can aggregate several machines; current error envelopes omit the machine field. The activity SSE
stream uses [activity-stream-v1](activity-stream-v1.md), while WebSocket attach uses the binary/control
and optional event frames documented in [Terminal input and
attach](serve-terminal-v1.md#terminal-input-and-attach); neither streaming transport uses the ordinary
JSON envelope.

Auth is a bearer token (`--token-stdin`, `--token`, or `AGENT_SESSION_TOKEN`) on the activity stream plus
all write and attach endpoints, compared without an early-exit on the token bytes; when no token is
configured (or it is empty) those endpoints fail closed (503). Prefer `--token-stdin` for launcher
integrations so token material does not appear in process arguments. It reads one trimmed token from
stdin, rejects empty input, rejects multiple newline-separated tokens, and rejects input over 8192 bytes.
`--token-stdin` conflicts with `--token`; both forms avoid printing token material in errors. Use a
strong, high-entropy token.

`--token` and `AGENT_SESSION_TOKEN` remain accepted compatibility inputs.
`--token` exposes the bearer to same-host process-argument inspection. The
environment form can be inherited by managed tmux/provider children because the
daemon does not currently scrub it before launch, which would grant those
children machine-operator authority. Deployments that create sessions must use
`--token-stdin` from a private, non-exported credential source.

## Account brokers

See [Account brokers](serve-accounts-v1.md#account-brokers).

### Codex account broker

See [Codex account broker](serve-accounts-v1.md#codex-account-broker).

### Claude account broker

See [Claude account broker](serve-accounts-v1.md#claude-account-broker).

## Launch profiles

Server-owned launch profiles are configured with
`AGENT_SESSION_LAUNCH_PROFILES`, a JSON array. For example:

```json
[{"id":"custom-claude","label":"Custom Claude","agent":"claude","agent_bin":"/opt/agent/bin/custom-claude","provider_config_dir":"/srv/agent/claude","readiness_args":["--check"],"auto_resume_supported":false}]
```

The daemon rejects malformed, duplicate, relative-path, or over-bounded configuration at startup. A
profile is advertised only when its executable is an executable regular file, its optional provider
config root is a directory, and the optional readiness argv exits successfully within two seconds. The
readiness argv always runs against `agent_bin`; no shell is involved. Profile discovery probes are
single-flight; concurrent session-list reads share the same in-flight result, while a later read probes
fresh. Profile paths and readiness details never enter HTTP responses.

The safe id and any configured private provider root persist with the runtime and its durable resume
sidecar, so exact binary and transcript discovery survive daemon restarts. Both daemon-managed and
standalone `agent-session resume <managed-id>` launches pin the persisted provider root, when present, in
the provider-specific environment before invoking the durable launcher. The daemon resume endpoint
additionally requires the same id, base agent, executable, optional config root, and auto-resume
capability to remain present in the current server registry.

It runs the readiness argv from the current profile and requires that probe to succeed, but does not
persist or compare the launch-time readiness argv. Removing the profile or changing a persisted identity
field revokes resume through that endpoint; changing only the readiness argv is accepted when the new
probe succeeds. Because standalone resume cannot enforce the live registry, the daemon does not advertise
it as a managed copy action. Set `auto_resume_supported` only when the profile has authoritative usage
semantics for its provider; the default is fail-closed `false`.

A profile with this capability disabled can still create its initial managed Codex thread and turn; it
does not gain automatic continuation support.

The retired base agent `hermes` is refused: serve does not start with a
profile that names it.

DSH launch profiles are retired. Serve configuration that names `agent = "dsh"`
is rejected with an actionable error; remove the DSH profile before starting
the daemon. DSH sessions cannot be created through the serve API.

## Trust model

The daemon binds loopback and *refuses* a non-loopback bind unless `--allow-non-loopback` is passed,
because it drives a remote shell. `GET /healthz`, `GET /sessions`, `GET /usage`, `GET
/sessions/{id}/glance`, `GET /sessions/{id}/buffer`, and `GET /sessions/{id}/auto-resume` are
intentionally open on the bind address. Those routes can expose working directories, recent prompts, pane
content, provider usage, auto-resume state, and the server-global tmux clipboard. Loopback blocks remote
access but does not authenticate same-host principals; the raw daemon therefore requires a trusted
single-user host or an equivalent local access-control boundary.

Path-bearing reads (`workdirs`), activity streaming, writes, and attach require the bearer token. The
provider hook ingress instead requires the session's own capability and a direct loopback peer; it grants
no operator authority. Front the daemon with the agent-console edge (which applies its own auth) and do
**not** `tailscale serve` the raw serve port; expose only the edge, tailnet-only, no funnel. Browser
WebSocket clients cannot set an `Authorization` header, so the edge must proxy the attach and inject the
bearer server-side — never put the token in the `ws://` URL/query.

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

Federation is specified in [coordination](coordination-federation-v1.md#cross-host-mailbox-federation-v1).
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
message as the session engine. When a failed create retains its session record,
error `details.session_id` identifies it and `details.startup` carries the same
safe startup projection as list and glance. Incomplete cleanup also keeps the
bounded `details.cleanup` projection. Clients can address the retained session
directly; its presence does not turn the failed operation into success.
