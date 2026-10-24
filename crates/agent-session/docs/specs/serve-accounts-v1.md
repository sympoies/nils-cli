# Serve account brokers and switching v1

Part of [Serve API v1](serve-api-v1.md). Its scope and shared rules also apply here.

## Account inventory

- `GET /codex/accounts` — authenticated nickname-only account inventory from the configured host
  credential broker: `{ "machine", "provider": "codex", "accounts": [{ "account", "label"?, "plan"? }],
  "selection_strategies", "readiness" }`. `selection_strategies` lists the broker-advertised selectors
  the daemon understands (`current_default`, `default_with_capacity`, `next_with_capacity`); `provider`
  and `selection_strategies` are additive. The `readiness` projection reports whether the installed Codex
  version meets the minimum app-server floor and currently advertises Unix listen support, with only a
  canonical provider version and stable safe reason code.

  A capable CLI also needs a usable private runtime directory (see [Session
  creation](serve-session-create-v1.md#session-creation)); otherwise readiness reports `supported: false`
  with `codex-app-server-runtime-dir-unavailable`, `codex-app-server-runtime-dir-unsafe`, or
  `codex-app-server-socket-path-too-long`. Newer stable Codex releases are accepted by capability instead
  of an exact-version allowlist; exact protocol attention remains limited to explicitly audited versions
  and otherwise falls back to hook authority. The response never contains access tokens, ChatGPT account
  ids, auth paths, or broker diagnostics. Without a configured broker the route returns `409
  codex-account-unsupported`.
- `GET /claude/accounts` — authenticated nickname-only Claude account
  inventory from the configured Claude account broker:
  `{ "machine", "provider": "claude", "accounts": [{ "account", "label"?,
  "plan"? }], "selection_strategies" }` (`provider` is additive). It never
  contains credentials or account directory paths. Without a configured broker
  the route returns `409 claude-account-unsupported`.

## Account switch route

- `PUT /sessions/{id}/account` accepts `{ "account": "nickname", "expected_session_incarnation":
  "launch-id" }` for a serve-managed Codex app-server runtime or a broker-bound Claude session; the
  response carries the provider's `codex_account` or `claude_account` projection.

  Both providers share these outcomes: an invalid nickname is `400 invalid-<provider>-account`; a
  nickname the broker does not list is `400 <provider>-account-unknown` (re-selecting the currently bound
  account is always accepted, so it can cancel a queued switch); a stale `expected_session_incarnation`
  is `409 <provider>-account-session-incarnation-conflict`; a daemon or session without that provider's
  broker is `409 <provider>-account-unsupported`. Claude applies a switch by relaunching; see the [Claude
  account broker](serve-accounts-v1.md#claude-account-broker).

  For Codex, at the authoritative `waiting` boundary, the daemon applies the account immediately without
  recreating tmux or resuming the provider conversation. While a turn is `working`, it instead stores an
  additive durable `next` intent and leaves `selected_account` unchanged until that intent applies
  successfully. This also applies to an unbound runtime that inherited the global default: it may queue
  its first explicit account when the request includes `supports_unbound_account_queue: true`, without
  claiming that account is already applied. The opt-in keeps daemon-first and edge-first rolling upgrades
  fail-closed.

  The public `next` projection contains only the desired nickname, revision, `queued` / `applying` /
  `failed` state, and a safe failure reason. A successful mutation response also echoes the current
  `session_incarnation`, allowing an intermediary to verify an unbound queued outcome against the request
  fence without exposing credential metadata. The control loop drains a queued intent after the live
  app-server reports no in-progress turn for the bound thread, even when the hook-derived activity
  projection has not yet caught up to `waiting`.

  The TUI proxy serializes account mutation with every forwarded `turn/start` until its matching provider
  response, so an accepted-but-not-yet-observed turn cannot cross that idle boundary. It resolves
  credentials through the host broker and sends Codex `account/login/start` with `chatgptAuthTokens`.
  Success flips the durable binding and clears `next`; failure preserves the applied binding and marks
  the intent failed. Prompt, terminal-input, and auto-resume submission paths fail closed while a
  selected binding is `pending` or `failed`, or while any live or malformed next intent exists, so the
  next accepted prompt uses the newly selected account.

  An interrupted `applying` intent is re-queued when the session runtime restarts. The local
  `agent-session account switch` CLI shares this route's code path. Without the daemon's in-process Codex
  control it always takes the durable `next` path above, which the control loop applies before the next
  prompt. With the unbound opt-in, a binding that is `failed`, `pending`, or malformed (for example after
  a daemon restart) also accepts a queued explicit account; the binding stays fenced until the daemon, at
  `waiting`, promotes the intent into this route's immediate switch with the same incarnation fence.
  Promotion rechecks the daemon's broker roster before changing the binding or consuming the intent;
  a removed account leaves both unchanged. Literal terminal text is also refused before paste or keys
  when the Codex binding is not bound to this runtime, including text without a submitting Enter.
  Control-only non-submitting keys remain available for dialogs.

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

For Codex `account switch`, the valid serve-recorded broker command takes precedence even when the
caller's broker variable is set, so repair membership is checked against the configured serve roster.
Other commands and Claude retain the caller-first adoption policy. Re-selecting the account already
bound to the current runtime still cancels a queued switch without requiring roster membership.

The per-provider differences are the protocol and what the broker returns.

### Codex account broker

The Codex broker speaks `agent-session.codex-auth-broker.v1`, without a `--provider` argument. The daemon
invokes `list`, `resolve`, or a bounded `select` request. `select --strategy current_default` returns the
configured nickname matched by the live default; `select --strategy next_with_capacity --after <nickname>
[--exclude <nickname>]...` walks configured order once and returns only a fresh, network-confirmed usable
account. `list` returns public `accounts`, while `resolve` returns the exact nickname plus
`access_token`, `chatgpt_account_id`, and optional `plan`; tokens are resolved on demand and kept in
memory. The additive list field `selection_strategies` advertises these selectors.

Choosing the automatic default for a new session is intentionally best effort: a daemon paired with an
older broker that omits the field, or a broker that cannot list, leaves the new session unbound on the
host login instead of failing the create. An explicit account, an account switch, and a selector failure
after capability is advertised remain fail closed.

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
  `next: {account, revision, state: "queued" | "failed", blocking_reason?, failure_reason?}`). It is omitted for other
  providers and for Claude sessions without binding state on a daemon without
  a Claude broker. It never contains the directory path.
- Switch: `PUT /sessions/{id}/account` with `{ "account", "expected_session_incarnation" }` on a Claude
  session durably queues `agent-session.claude-account-next.v1`; requesting the bound account cancels a
  queued intent. A nickname the broker does not list is refused with `claude-account-unknown`. Claude
  Code has no live credential swap, so the switch is applied only by a relaunch: when the session is
  running and its turn is `waiting`, the daemon first materializes and validates the new account
  directory.

  A refusal returns `claude-account-switch-refused` (HTTP 422, with the broker's `cause` code) before
  anything is stopped or written, so the running session, its binding, and any previously queued intent
  stay unchanged. Otherwise it stops the runtime through the verified stop path and resumes the same
  conversation (`--resume <session-id>`) in the new account directory, then returns the new
  `session_incarnation`. While a turn is busy, the intent stays queued. The daemon rechecks queued Claude
  intents every two seconds and applies them after a provider hook establishes `waiting` (including the
  Stop completion quiet window).

  This explicit switch is independent of auto-resume enablement and recovered auth incidents. An external
  switch reissued while idle applies synchronously. A stopped session retains the intent for its next
  explicit resume. Queued intents expose a path-free `next.blocking_reason`: `turn_busy`,
  `turn_needs_input`, `turn_starting`, `turn_state_unavailable`, `session_stopped`,
  `session_status_unknown`, or `self_restart_deferred`. A daemon preflight refusal records the broker's
  cause code there; a refused stop records its error code. The intent and revision remain queued for
  retry, and nothing relaunches until preflight and verified stop succeed.

  Repeated broker or stop refusals retry at most once every 30 seconds for an unchanged intent and
  runtime, with a warning only when the refusal reason changes. A new intent revision, changed turn-state
  blocker, or replacement runtime clears that delay. Explicit idle reissues still run immediately. After
  a verified stop, the switch retires the stopped runtime's coordination incarnation (its heartbeat
  writer died with it) before resuming, so the resume is not refused as a live prior incarnation.

  If the resume still fails and the current runtime is proven stopped, the response is
  `claude-account-switch-resume-failed` (HTTP 422) with `details: {id, session_state: "stopped",
  next_account, cause, recovery}`: the next account is retained with `state: "failed"` and
  `failure_reason` carrying the underlying error code. The account projection's state is `failed`, and
  the session's `startup` projection reports `claude-account-switch-resume-failed`, so later Console
  reads expose the failure. `recovery` names the `agent-session resume <id>` command that applies it.

  If replacement cleanup remains live or unverified, the response preserves the original termination or
  rollback error instead of claiming a stopped session. The account attempt is still marked failed with
  that code; startup reports `claude-account-switch-cleanup-incomplete` with `retry_safe: false`; the
  durable startup record retains bounded cleanup state. The local `agent-session account switch` CLI
  shares this path; run from inside the session's own tmux session it only queues, since restarting the
  runtime that hosts it would stop the switch mid-way. Automatic authentication
  failover requires the separate [broker policy preflight](auth-failover-v1.md).
- History: catalog scans attribute each physical transcript once, by its
  canonical path, so an account directory whose `projects/` links to the
  shared `~/.claude/projects` is not counted twice.
