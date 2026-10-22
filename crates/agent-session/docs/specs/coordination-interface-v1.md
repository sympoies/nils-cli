# Coordination CLI and HTTP v1

Part of [Session Coordination V1](session-coordination-v1.md). Its scope and shared rules also apply here.

## CLI contract

All commands support the global `--state-dir` and command-local `--format
text|json`. `start` and `run` accept `--coordination-mode
advisory|enforce|off`, defaulting to `advisory`. High-level commands infer the
current session and capability only from trusted managed runtime projection.
Raw owner commands retain explicit `--session`, except `message inbox`, whose
omitted recipient defaults to `AGENT_SESSION_ID`. Explicit recipients take
precedence. `--capability-file` defaults only from the trusted managed
environment; the matching capability is always authenticated.

Every leaf command has its own CLI envelope identity, for example
`cli.agent-session.message-inbox.v1`, `cli.agent-session.broker-status.v1`, and
`cli.agent-session.work-context-admit.v1`.

```text
agent-session work-context status
agent-session work-context set [--if-absent] [--summary TEXT] [--intent NAME] [--tier direct|issue|program|program/dispatch] [--repository OWNER/REPO] [--path PATH]... [--issue N]... [--pr N]...
agent-session work-context clear
agent-session work-context advise [--targets-file JSON]
agent-session work-context acknowledge [--for DURATION]

agent-session work-context claim --session ID --file JSON --capability-file FILE --idempotency-key KEY [--if-revision N]
agent-session work-context show --session ID
agent-session work-context check (--self --capability-file FILE | --session ID | --candidate JSON) [--allow-incomplete]
agent-session work-context renew --session ID --claim UUID --if-revision N --capability-file FILE --idempotency-key KEY
agent-session work-context release --session ID --claim UUID --if-revision N --capability-file FILE --idempotency-key KEY
agent-session work-context admit --session ID --claim UUID --if-revision N --targets-file JSON --operation KIND --execution-token-file FILE --capability-file FILE --idempotency-key KEY
agent-session work-context complete --session ID --lease UUID --if-revision N --execution-token-file FILE --outcome pass|fail --capability-file FILE --idempotency-key KEY
agent-session work-context reconcile --session ID --lease UUID --if-revision N --proof-file JSON --capability-file FILE --idempotency-key KEY

agent-session broker status --session ID [--capability-file FILE]
agent-session broker adopt --session ID --capability-file FILE --proof-file JSON --idempotency-key KEY
agent-session broker reconcile --session ID --capability-file FILE --proof-file JSON --operation UUID --if-revision N --attest-inactive --idempotency-key KEY
agent-session readiness --format json

agent-session message send --from ID --to ID --body-file FILE [--capability-file FILE] --idempotency-key KEY [--reply-to UUID] [--expires-in DURATION]
agent-session message inbox [--session ID] [--capability-file FILE] [--state unread] [--cursor CURSOR] [--limit N]
agent-session message show --session ID --message UUID [--capability-file FILE]
agent-session message ack --session ID --message UUID --if-revision N [--capability-file FILE] --idempotency-key KEY
agent-session message reply --session ID --message UUID --if-revision N --body-file FILE [--capability-file FILE] --idempotency-key KEY
agent-session message wait --session ID --message UUID --if-revision N --timeout DURATION [--capability-file FILE]
agent-session message wait --session ID --any-unread [--newer-than CURSOR] --timeout DURATION [--capability-file FILE]
agent-session message reminder --session ID [--capability-file FILE]
```

JSON uses the existing `cli.agent-session.<command>.v1` success/error envelope
convention. For `message wait --any-unread`, exit `0` means unread metadata
arrived and exit `69` (`EX_UNAVAILABLE`) means the bounded timeout elapsed;
other failures retain their normal CLI error exit codes. The result includes a
recipient- and incarnation-bound opaque cursor; pass it as `--newer-than` on a
later wait to exclude message IDs already returned by the prior result. New
unread messages qualify regardless of their wall-clock timestamp or ID order.
Recoverable timeout and notification errors include typed retry guidance.
Errors never echo body, capability, request JSON, local private paths, or peer
summary.

### Managed-session readiness

`agent-session readiness` authenticates the capability supplied by
`AGENT_SESSION_CAPABILITY_FILE` against one ready, heartbeat-fresh broker and
its current session-record incarnation. It requires no orchestration run,
assignment, or work-context claim and applies in every coordination mode.
It then verifies `AGENT_SESSION_CHECKPOINT_FILE` equals the broker's expected
incarnation-bound checkpoint path. The file must be a regular file owned by the
current user, mode `0600`, with one link and no final symlink. The command does
not create, repair, read, or write checkpoint contents.

The success envelope is `cli.agent-session.readiness.v1`. Its private
integration payload has schema `agent-session.runtime-readiness.v1`,
`ready: true`, and string fields `session_id`, `session_incarnation`, and
`checkpoint_file`. Consumers must validate those fields against their immutable
managed-runtime projection and independently resolve the trusted helper;
selectors and a successful JSON shape alone are not authentication. Do not
copy this private proof into public summaries or retained diagnostics.

Missing, stale, revoked, or untrusted credentials fail with
`coordination-unauthorized`; a missing, mismatched, or untrusted checkpoint
fails with `runtime-checkpoint-unavailable`. Both return exit `65`, `ok: false`,
and bounded recovery details: `retryable: false`,
`next_action: "resume-or-restart-managed-session"`, and a `recovery` object with
the same `action`. Errors disclose no capability value or private path.
Transport/store failures retain their existing codes and fail closed.
Readiness does not acquire a baseline claim or authorize mutations; callers
retain their own baseline work-context and admission checks.

## HTTP coverage

The loopback server exposes the raw work-context, broker, and mailbox library
operations below. The high-level self-targeting CLI conveniences
`work-context status|set|clear|advise|acknowledge` are CLI-only; they derive
trusted session and checkout state from the managed runtime and do not have
one-for-one HTTP routes.

```text
GET  /sessions/{id}/work-context/v1
POST /sessions/{id}/work-context/check/v1
POST /coordination/work-context/check/v1
POST /sessions/{id}/work-context/claim/v1
POST /sessions/{id}/work-context/renew/v1
POST /sessions/{id}/work-context/release/v1
POST /sessions/{id}/work-context/admit/v1
POST /sessions/{id}/work-context/complete/v1
POST /sessions/{id}/work-context/reconcile/v1
GET  /sessions/{id}/broker/v1
POST /sessions/{id}/broker/adopt/v2
POST /sessions/{id}/broker/reconcile/v2
POST /sessions/{id}/broker/adopt/v1       (transition alias)
POST /sessions/{id}/broker/reconcile/v1   (transition alias)
POST /sessions/{id}/operations/{lease_id}/operator-reconcile/v1
                                               (HTTP-only, server Bearer)
POST /sessions/{id}/activity/provider-turn/operator-reconcile/v1
                                               (HTTP-only, server Bearer)
GET  /sessions/{id}/messages/v1
POST /sessions/{id}/messages/v1
GET  /sessions/{id}/messages/{message_id}/v1
POST /sessions/{id}/messages/{message_id}/ack/v1
POST /sessions/{id}/messages/{message_id}/reply/v1
GET  /sessions/{id}/messages/{message_id}/wait/v1
```

For the raw operations that both transports expose, CLI and HTTP share one
implementation, canonicalization, authorization, idempotency, error codes,
limits, and privacy projection. HTTP public reads require only the server
operator bearer; owner/mailbox mutations additionally require the exact session
capability. Conflicting selectors are rejected. Wait cancellation closes
without changing message state.

For `POST /sessions/{id}/messages/v1`, `{id}` is the recipient. The required
`X-Agent-Session-Capability` determines the sender; the JSON body contains only
`body`, `idempotency_key`, optional `reply_to`, and optional `expires_in`, and
rejects a `to` redirect selector. The session check body contains only
`self_selector` (default false) and `allow_incomplete`; candidates are accepted
only by the registry-level check route.

Successful HTTP send and reply envelopes carry the same content-free
`notification` state/generation/reason projection as CLI and set
`controller_available: true`. The response schedules work only; it does not
promise immediate delivery or mark a message read.

## Public list and glance additions

List and glance may add only:

- `coordination_mode`;
- `work_context_state`;
- `claim_id`;
- `claim_expires_at`;
- `unread_message_count`;
- `mailbox_quota_warning` (only near/at recipient quota; content-free counts,
  limits, status and fixed recipient recovery instructions);
- `coordination_conflict_severity`;
- `coordination_available`.

Existing fields, including `cwd`, do not change. New fields never embed
the full context, body, capability, incarnation, host/user, checkout path, or
private store location.

## Stable failure codes

The v1 surface distinguishes at least:

- `coordination-unavailable`, `coordination-broker-start-timeout`,
  `coordination-broker-lost`, `coordination-lock-timeout`;
- `coordination-unauthorized`, `session-incarnation-conflict`,
  `claim-revision-conflict`, `message-revision-conflict`;
- `unsupported-work-context-version`, `invalid-work-context`,
  `invalid-scope`, `uncovered-mutation-scope`, `incomplete-conflict-view`;
- `claim-conflict`, `idempotency-key-reused`, `operation-in-progress`,
  `operation-reconcile-pending`, `broker-replacement-grace`,
  `quota-exceeded`, `rate-limited`,
  `cursor-invalid`, `wait-timeout`, `wait-cancelled`;
- `mailbox-body-source-invalid`, `mailbox-body-invalid`, `mailbox-body-too-large`, `reply-depth-exceeded`,
  `message-expired`, `message-not-found`;
- `coordination-store-untrusted`, `coordination-store-corrupt`.
- `session-not-managed`, `not-in-repository`, `repository-unavailable`,
  `invalid-acknowledgement-duration`.

Usage errors exit 64, data/contract errors use the workspace data exit code,
and runtime/storage failures use the runtime exit code.

## Validation matrix

Release readiness requires:

- additive old-record coverage proving missing `coordination_mode` defaults to
  advisory;
- automatic presence coverage for same worktree, same repository/different
  worktree, optional context, off peers, stale brokers, and target exit;
- self-targeting status/set/clear/acknowledge coverage proving no manual session
  ID, capability path, context file, revision, or idempotency key is needed;
- self-targeting set coverage distinguishing a safely resolved cwd outside Git
  (`not-in-repository`) from symlinked, missing, or otherwise unprovable cwd
  boundaries (`uncovered-mutation-scope`) and unresolved checkout origins
  (`repository-unavailable`);
- advisory/enforce/off and unmanaged cross-product acceptance;
- table/property coverage for canonicalization, closed scopes, peer selection,
  conflict precedence, and keyed fingerprint epochs;
- concurrent process coverage proving exactly one definite claimant;
- capability, incarnation, revision, idempotency, and target-subset negatives;
- fake-clock/process coverage for long operations, broker loss/adoption,
  missed completion, replacement, and cleanup;
- CLI/HTTP parity for every shared raw operation, selector combination, error,
  wait, and cancellation outcome, plus explicit coverage that self-targeting
  conveniences remain CLI-only;
- mailbox permissions, limits, rate, pagination, retention, flood/restart, and
  privacy canaries;
- held-launch crash injection at record, pane, identity, credential, broker
  spawn/readiness, and exec boundaries;
- notification generation migration/coalescing, exact-byte golden, body
  non-interference, controller restart, racing wakeups, Codex acknowledgement,
  Claude Stop/debounce and detached fencing, transcript acceptance,
  attempt-unknown reconciliation, busy, replaced, unsupported, failure, and
  crash windows;
- unchanged established lifecycle/list/send/server regression suites and completion
  freshness/parity checks.
