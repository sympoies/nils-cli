# Serve mailbox and automation v1

Part of [Serve API v1](serve-api-v1.md). Its scope and shared rules also apply here.

## Mailbox delivery and automatic recovery

- `POST /sessions/{id}/messages/v1` and `POST /sessions/{id}/messages/{message_id}/reply/v1` persist
  unread mail and schedule an eventual body-free mailbox notification. Their coordination result adds `{
  state, generation, notified_generation, last_reason?, controller_available }` under `notification`; the
  active server sets `controller_available: true`. Delivery is controller-owned and may occur later when
  the exact managed Codex or Claude incarnation reaches its fenced safe-input boundary. The response
  never implies that the message was read, and it exposes no mailbox body, receipt key, capability,
  incarnation, or provider turn identifier.

  The full state, adapter, and privacy contract is in [Session coordination
  v1](coordination-mailbox-v1.md#notification-ownership). Codex app-server runtimes use acknowledged
  structured submission: idle turns use `turn/start`, while an authoritative active turn uses
  `turn/steer` with its exact `expectedTurnId`. The control connection transiently maps the durable
  projected turn fence back to the matching raw active turn id; raw ids never enter session documents or
  API projections. Notice-capable recipients receive the canonical `mailbox.wake`
  envelope under the existing writer gate, with coalesced unread counts.
  Explicitly unsupported recipients retain the plain reminder; failed/uncertain
  notice attempts never fall back or repeat. Long-running work observes the body-free reminder at the provider's next model
  checkpoint. Both paths retain the exact incarnation and no-claim/no-operation generation fence.

  Terminal-backed Codex and Claude runtimes use the same notification-generation CAS plus exact
  incarnation, idle-turn, live-runtime, and detached-session rechecks before one fixed body-free prompt
  and a single Enter. The terminal attempt boundary additionally requires an authoritative broker with no
  active claim or active/uncertain operation. Its durable per-session `attempting` state fences new claim
  and operation admission through submission without holding the global coordination registry lock across
  terminal I/O. Transcript observation determines acknowledged versus unknown outcome.
- A daemon configured with `[auto_resume] default_enabled = true` (or
  `AGENT_SESSION_AUTO_RESUME_DEFAULT_POLICY`; see the
  [serve daemon runbook](../runbooks/serve-daemon.md#configuration-file))
  opts every session that `POST /sessions` creates or imports into auto-resume
  with the configured `default_recovery_policy` (`wait_for_reset` when
  omitted), as if the client had sent that `PUT` right after creation, and the
  create response already reports that view. It is applied once, only when the
  session has no durable auto-resume state, so an explicit `PUT` or `DELETE`
  always wins. Sessions that cannot report an authoritative capacity signal, or
  cannot honor the policy (for example `next_account_then_resume` without a
  bound Codex account), stay disabled; existing sessions are unchanged.
- Local operators can use `agent-session auto-resume <id> --format json`
  to read the same projection, or `--enable` / `--disable` to change it without
  a serve bearer token, using the state directory's existing local session
  access. `--policy wait_for_reset|next_account_then_resume` requires
  `--enable` and defaults to `wait_for_reset`. The local command uses the same
  locked policy setter and support guards as `PUT`; unsupported runtimes,
  including standalone Codex TUI sessions, cannot be enabled. It configures
  durable recovery; a running serve daemon still owns scheduling and submission.
- Every session view additively includes `auto_resume` using `agent-session.auto-resume.v1`. `GET
  /sessions/{id}/auto-resume` reads that status and is open on loopback; `PUT` with `{ "enabled":
  true|false, "recovery_policy": "wait_for_reset"|"next_account_then_resume" }` opts a supported session
  in or out, and `DELETE` durably cancels pending work. The policy defaults to `wait_for_reset`. Account
  failover is accepted only for a bound, broker-backed Codex session. It selects the next configured
  account with fresh confirmed capacity, applies that account through external auth, and only then enters
  the existing exactly-once continuation claim.

  The structured rejection remains authoritative when the current percentage windows are still open,
  because workspace-credit exhaustion is independent from those windows. Exhausted and already-attempted
  accounts are not revisited in the same recovery chain, including when a failed automatic continuation
  produces a new provider turn; a later manual input starts a fresh chain. When an explicit account
  switch wins the race with the rejected turn, the input fence identifies the older account and binding
  revision, and the daemon may continue once a different newly bound account reports authoritative open
  usage without first invoking the broker again.

  Pre-upgrade recovery state that lacks either part of that input-binding identity fails closed with
  bounded `state_unavailable` retries: it does not invoke the broker, queue an account switch, or submit
  a continuation. When no candidate is trustworthy, a current exhausted percentage window waits for its
  confirmed reset; an open current window reports `no_account_available` after bounded discovery retries
  instead of mislabeling the provider rejection as `usage_window_not_exhausted`. Both mutations require
  the bearer token. Claude Code is supported through its authoritative structured `StopFailure.error ==
  "rate_limit"` signal.

  It is the only Claude `StopFailure.error` value that counts as capacity: `server_error` (a provider
  overload or outage), `authentication_failed`, `oauth_org_not_allowed`, `billing_error`,
  `invalid_request`, `max_output_tokens`, and any unknown value never arm recovery, and the fixture
  `tests/fixtures/activity/auto-resume-failures.jsonl` pins each of them. Fresh interactive Codex
  sessions created through the serve API are also supported when the installed CLI capability probe
  selects the app-server v2 runtime.

  The daemon consumes the live metadata-only protocol and requires an exact bound thread/turn with
  terminal `status == "failed"` plus `codexErrorInfo == "usageLimitExceeded"` or the internal
  `serverOverloaded` capacity cause. Capacity recovery preserves the public v1 projection, waits 30, 60,
  120, 300, and 600 seconds across at most five submitted attempts, and uses this fixed control-channel
  message: `The selected model was at capacity, interrupting the previous turn. Please continue from
  where you stopped.` Standalone/raw Codex TUI, imported Codex conversations, and resumed pre-app-server
  Codex sessions remain unsupported. Terminal text and assistant output are never treated as authority.

  The response object is `{ schema_version, supported, enabled, recovery_policy, state, attempts,
  scheduled_at?, failure_reason?, failure_class? }`. `attempts` is the number of capacity-recovery
  continuations the current chain has submitted (`0` before the first, and for usage-exhaustion
  recovery). `failure_class`, present while a recovery chain exists, is `usage_exhaustion` or
  `provider_capacity`; clients render any other value as a generic failure. `scheduled_at`, when present,
  is an RFC 3339 timestamp. The v1 `state` values are `disabled`, `enabled`, `armed`, `scheduled`,
  `switching_account`, `checking`, `resumed`, `cancelled`, `transient_failure`, and `terminal_failure`.

  The allowlisted `failure_reason` values are `state_unavailable`, `manual_input`, `usage_unavailable`,
  `usage_window_not_exhausted`, `exhausted_reset_unavailable`, `session_state_changed`,
  `submission_outcome_unknown`, `provider_unsupported`, `account_switch`, `no_account_available`,
  `account_switch_failed`, and `scheduler_error`, `control_unavailable`, `account_changed`, and
  `capacity_retry_exhausted`. Consumers must preserve the object but render unknown future state or
  reason values as a safe generic unavailable/failure condition; they must not infer permission to submit
  from an unknown value.

  `scheduled_at` is present only while the daemon has a next scheduled wake, either for a provider reset,
  an unknown-reset continuation probe, or bounded provider-capacity backoff. `failure_reason` is present
  only when the latest transition records a safe operational reason.
- The daemon owns scheduling. It waits for the latest reset among all exhausted windows, adds bounded
  deterministic jitter, re-collects usage at wake, checks that the session activity revision is still
  eligible, and durably claims the submission before sending one fixed product-owned continuation
  message. Capacity recovery skips usage collection but retains the same runtime, binding, activity
  revision, pending-attention, manual-input, health, and durable pre-submit fences. An indeterminate
  control submission is terminal and is never replayed after restart.

  When an authoritative Claude rate limit has no exhausted percentage window or future reset timestamp,
  the daemon keeps the claim scheduled and uses a low-frequency continuation probe (backing off through
  five, fifteen, thirty, and sixty minutes after repeated structured rate-limit failures) until it
  succeeds or a user/session-state change cancels it. On upgrade, the daemon recovers the exact
  pre-upgrade terminal `usage_window_not_exhausted` state only while its blocked activity revision is
  still eligible; other terminal states remain fail-closed.

  Restart recovery scans pending records; duplicate events/ticks cannot submit twice, cancellation is
  serialized against wake-up, and bounded retry for non-authoritative usage or scheduler failures ends in
  an observable terminal failure. Claude usage checks use the existing provider helper. Codex app-server
  sessions use `account/rateLimits/read` on their bound control connection and submit the continuation
  with `turn/start`; only a response carrying the acknowledged turn id counts as success. A timeout or
  disconnect after the durable claim is terminal `submission_outcome_unknown` and is never replayed.
