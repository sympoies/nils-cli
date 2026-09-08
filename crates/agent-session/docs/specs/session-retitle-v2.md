# Session Retitle v2

Session retitling is owned by the `agent-session serve` daemon. It observes the
authoritative provider turn state, reads bounded provider transcript history,
asks a configured primary title provider (or one bounded fallback) for a strict
decision, and commits that decision behind session-incarnation, title-revision,
activity-revision, and provider-turn fences.

## Capability and routes

The session list advertises `data.capabilities.session_retitle_v2 = true`.
Both routes use the authenticated `cli.agent-session.serve.v1` envelope and put
the daemon-owned machine identity in `data.machine`.

- `GET /retitle/readiness` returns `data.retitle` with schema
  `agent-session.session-retitle.readiness.v2` and capability
  `agent-session.session-retitle.v2`.
- `POST /sessions/{id}/retitle` accepts schema
  `agent-session.session-retitle.request.v2` and returns `data.retitle` with
  schema `agent-session.session-retitle.v2`.

Readiness fields are `status` (`ready`, `degraded`, or `unavailable`),
`reason_code`, `next_action`, optional `provider_kind`, `model_label`, `account`,
and `plan`, plus `context_capabilities`. Provider kinds are
`codex_subscription`, `openai_compatible`, and `command`.

Stable readiness reasons are `ready`, `provider_not_configured`,
`config_invalid`, `account_broker_unavailable`, `account_missing`,
`api_key_missing`, `provider_command_unavailable`, `fallback_ready`, and
`legacy_command_provider`. Stable readiness actions are `none`,
`configure_provider`, `configure_account_broker`, `select_account`,
`set_api_key`, `install_provider_command`, `restore_primary`, and
`migrate_provider`.

## Mutation request and response

The strict request object contains:

```json
{
  "schema_version": "agent-session.session-retitle.request.v2",
  "trigger": "manual",
  "idempotency_key": "manual-retitle-0001",
  "expected_session_incarnation": "opaque-launch-id",
  "expected_title_revision": 3
}
```

Automatic triggers are `initial`, `prompt`, and `completion_recovery`; they
also require `expected_activity_revision` and `expected_provider_turn_id`.
Manual requests must omit those two provider-turn fences. An idempotency key is
8–128 printable, non-space ASCII bytes and is permanently bound to the complete
request digest in the bounded receipt window.

The response reports `outcome` (`committed`, `unchanged`, or `replayed`),
`changed`, `trigger`, `provider_kind`, optional `processed_turn_id_hash`, a
content-free `diagnostic_code`, bounded transcript `coverage`, and a `session`
projection containing `title`, `title_state`, `title_revision`, and
`session_incarnation`. For decisions committed by this version,
`provider_kind` identifies the provider that produced the decision and is
retained for idempotent replays. An earlier receipt without provider attribution
uses the currently configured provider kind for response compatibility. It
never returns a prompt, transcript excerpt, raw
provider turn ID, provider model output, credential, or private path.

The daemon schedules the newest provider-confirmed current turn. If its prompt
observation races transcript persistence, the matching completion can schedule
one `completion_recovery` attempt. The deterministic key includes session
incarnation, provider turn ID, and trigger. Receipts and the last processed turn
hash live in the session record, so restart, resume, reload, and multiple clients
converge without duplicate provider decisions. An orphaned `in_progress`
receipt is recoverable only after the replacement daemon has acquired its
process-local per-session gate. Retryable automatic failures use one- and
two-second process-local backoff and a durable maximum of three provider
attempts per deterministic key. Terminal success, non-retryable failure, and
attempt exhaustion remain suppressed for the daemon lifetime; restart cannot
reset the durable provider-attempt bound.

Commit always reloads and rechecks every fence. A later activity revision is
accepted while the expected provider turn remains current, or after that turn
becomes the latest completed turn with no newer current turn. A different
current turn invalidates the decision. User-owned topics are immutable to
retitling. A manual retitle can repair an automatic topic. References are kept
only when they occur in authoritative user prompts, title components are limited
to 120 characters, and equal topic/activity text is deduplicated.
Complete injected `AGENTS.md` instruction regions and provider context blocks
are omitted before per-message bounds are applied. Authorization headers,
credential-shaped JSON lines, private-key markers, paths, and token-shaped
values are removed before any provider request is constructed.

## Provider configuration

Set one JSON object in `AGENT_SESSION_RETITLE_CONFIG`. Changes take effect when
the daemon restarts. Common bounds are `timeout_ms` 1000–120000,
`max_output_tokens` 1–4096, `max_concurrency` 1–8, `queue_size` 0–64, and
`context.max_chars` 1000–65536, `per_message_chars` 128–8192,
`recent_turns` 1–32. One `timeout_ms` deadline is shared by semaphore queue wait,
context preparation, and provider execution; a queued request does not receive
a second full provider timeout. The 120-second maximum stays inside Agent
Console's 125-second mutation transport deadline.

Automatic admission binds a minimum observed activity revision and provider
turn. Later progress within that same current turn (or its completed last-turn
projection when no newer turn exists) is accepted at admission and commit;
older revisions and newer provider turns fail closed. Automatic retry
identities remain stable per turn while request idempotency also binds the
refreshed revision, including both activity and title revisions, so a same-turn
conflict can recover without allowing concurrent progress events to fan out
into duplicate requests.

The primary object may contain one `fallback` provider object. Fallbacks cannot
nest or contain root-owned `max_concurrency`, `queue_size`, or `context` fields,
and the primary plus fallback `timeout_ms` values must total at most 120000. The
single total deadline covers queueing, context construction, account-broker
resolution, provider setup, the primary, and the fallback. A fallback is
attempted only for account, API-key, provider timeout,
availability, rate-limit, quota, or malformed-response failures. Queue,
context, session-incarnation, title-revision, activity-revision, provider-turn,
and durable-state failures remain fail-closed. If the primary is statically
unavailable but the fallback is configured and ready, readiness reports the
fallback provider with `degraded`, `fallback_ready`, and `restore_primary`.

Codex subscription uses the existing account broker and the supported Codex
app-server protocol. It supplies broker credentials through external auth in a
temporary `CODEX_HOME`; it never switches or rewrites the operator's global
Codex account and does not call undocumented ChatGPT HTTP endpoints.

```json
{
  "provider": "codex_subscription",
  "account": "sym",
  "codex_bin": "/absolute/path/to/codex",
  "model": "gpt-5.6-luna",
  "timeout_ms": 20000,
  "max_concurrency": 1,
  "queue_size": 8,
  "context": {"max_chars": 12000, "per_message_chars": 2000, "recent_turns": 12},
  "fallback": {
    "provider": "openai_compatible",
    "base_url": "http://127.0.0.1:1237/v1",
    "model": "qwen3.6-apex-compact",
    "timeout_ms": 100000,
    "json_response": true
  }
}
```

OpenAI-compatible configuration works with DeepSeek and local servers. The
daemon appends `/chat/completions` to `base_url` (or `/v1/chat/completions` when
the supplied base ends in `/v1`). `api_key_env` names an environment variable;
the key itself is not stored in JSON. Response bodies are streamed into a
fixed cap-plus-one reader and rejected before an oversized response can be
buffered in full.

```json
{
  "provider": "openai_compatible",
  "base_url": "https://api.deepseek.com",
  "model": "deepseek-chat",
  "api_key_env": "DEEPSEEK_API_KEY",
  "json_response": true
}
```

The compatibility provider runs a bounded argv directly, sends one JSON request
on stdin, and expects the strict decision JSON on stdout. It is deliberately
reported as `degraded` with reason `legacy_command_provider`.

```json
{"provider":"command","argv":["/absolute/path/to/title-command"]}
```

Mutation errors use stable codes and content-free details with `retryable`,
`next_action`, and `recovery.{strategy,safe_to_retry}`. Mutation actions are
`retry`, `wait_and_retry`, `refresh_session`, `wait_for_context`,
`configure_provider`, `configure_account_broker`, `select_account`,
`set_api_key`, `inspect_provider`, and `none`.

| Code | `next_action` | Recovery strategy |
| --- | --- | --- |
| `invalid-retitle-request` | `refresh_session` | `replace_request` |
| `retitle-provider-not-configured` | `configure_provider` | `configure_provider` |
| `retitle-config-invalid` | `configure_provider` | `replace_configuration` |
| `retitle-queue-saturated` | `wait_and_retry` | `bounded_backoff` |
| `retitle-context-unavailable` | `wait_for_context` | `retry_after_transcript_catchup` |
| `retitle-account-missing` | `select_account` | `refresh_account` |
| `retitle-api-key-missing` | `set_api_key` | `configure_secret` |
| `retitle-provider-timeout` | `retry` | `retry_request` |
| `retitle-provider-unavailable` | `retry` | `retry_request` |
| `retitle-provider-rate-limited` | `wait_and_retry` | `bounded_backoff` |
| `retitle-provider-quota-exceeded` | `wait_and_retry` | `wait_for_quota` |
| `retitle-provider-malformed-response` | `inspect_provider` | `repair_provider_output` |
| `idempotency-key-reused` | `refresh_session` | `replace_idempotency_key` |
| `session-incarnation-conflict` | `refresh_session` | `refresh_session` |
| `title-revision-conflict` | `refresh_session` | `refresh_session` |
| `retitle-turn-conflict` | `refresh_session` | `process_newest_turn` |
| `retitle-state-conflict` | `refresh_session` | `refresh_session` |
| `retitle-worker-failed` | `retry` | `retry_request` |
| `title-revision-overflow` | `none` | `none` |

`session-not-found` is the ordinary serve not-found contract. Any unexpected
storage or worker error is collapsed to `retitle-worker-failed`; private paths
and internal error strings are not forwarded.
