# Agent-session turn state contract

## Compatibility

`agent-session.turn-event.v1` is the local ingestion contract and
`agent-session.turn-state.v1` is the optional session-view contract. New
`agent-session` versions add `turn_state` and `runtime_started_at` to start,
resume, list, command, glance, and serve responses. Old records and damaged or
unsupported provider integrations remain readable; absent activity is omitted,
and corrupt activity degrades to `unknown` without breaking session lifecycle.

Consumers must ignore additive unknown fields. A future, unrecognized
`turn_state.schema_version` must be treated as unknown rather than interpreted
as a v1 phase.

## Provider authentication incidents

Claude prompt quarantine and same-account recovery are defined in the
[Claude auth recovery contract](specs/claude-auth-recovery-v1.md).

Claude `StopFailure.error=authentication_failed` and bound Codex app-server
`codexErrorInfo="unauthorized"` or a model HTTP error variant with
`httpStatusCode=401` produce `last_turn.provider_failure_kind="authentication"`.
Codex error notifications are observed even when `willRetry` is true or absent:
a structured Unauthorized/401 already proves an authenticated request failed,
independently of the retry policy. Reporting does not disable provider retries;
a later successful bound-thread completion settles the incident as `healthy`.
Failed-turn notifications provide the same incident evidence. MCP URL login
requests, HTTP 403, and ordinary message text are excluded. External-token
`account/chatgptAuthTokens/refresh` requests with `reason="unauthorized"` also
produce incidents; reused JSON-RPC IDs represent separate refresh occurrences.

List and board JSON expose an optional `auth_incident`, bound to the managed
session, runtime launch identity and generation. It contains a digest incident
ID, provider, optional configured account nickname, host `observed_at`, fixed
source (`claude_stop_failure`, `codex_app_server`, `codex_external_refresh`, or
`terminal_pattern`), confidence, status and notification receipts. Provider
errors, terminal bytes, prompts, credential values and credential paths never
enter the incident or notification body. Old-runtime incidents remain private
unless an `auto_settle` schedule owns the current carried quarantine. That
projection retains the incident's original runtime selectors; the schedule
identifies the current runtime receiving rechecks. Pending owner notifications
continue using the current authenticated runtime; private mailbox
journals reconcile earlier submissions before any resend. The bounded history
holds sixteen incidents and evicts only fully settled incidents. If all slots
have pending obligations, those obligations are preserved and new incidents are
refused with `degraded_queue_capacity`; the activity authentication failure kind
remains visible. Observation can retry after delivery frees a settled slot.

The daemon polls live sessions every 15 seconds, reconciles committed activity
when incident persistence was interrupted, and sends one incident message to
the session's effective parent. An open incident coalesces repeated evidence;
notification retries preserve the incident body and mailbox idempotency key.
A completed provider turn produces one final `healthy` recovery-result message.
Runtime replacement settles the old incident with `runtime_replaced`, without
claiming provider health. Historical delivery receipts must match the owner,
body and category. When an uncertain submission outlives the journal retention
window, reporting fails closed as `delivery_unknown` rather than resending a
possibly delivered incident.
External refresh produces `credentials_refreshed` or `refresh_failed`; refreshed
credentials alone do not claim a healthy model turn. Reporting failures do not
prevent the provider's existing refresh path from running. Sessions without an
owner route report `owner_unavailable` in the notification receipt.

A session with no effective parent (for example a root coordinator) cannot
receive its own incident, and input to a quarantined runtime stays refused.
The daemon instead appends one fixed-classification event per notification to
the daemon-independent observation spool, as described in
[control-plane observation](specs/control-plane-observation-v1.md): stage
`auth-incident`, code `auth-owner-unavailable` (or
`auth-recovery-owner-unavailable` for the final result), correlation
`sha256:` plus the `incident_id`, and for the incident a `recovery_action` that
names `lineage adopt` to give the session an owner. A consumer deduplicates on
the correlation and joins the typed incident from list or board JSON. Recording
the event is not delivery: `delivered_at` stays null and a later restored owner
route still delivers through the mailbox.

Codex same-account refresh additionally writes a private, token-free
`agent-session.codex-auth-recovery.v1` receipt store. Runtime incarnation and
generation fence the store; each operation hashes the connection scope and
provider JSON-RPC request ID and captures the selected account, binding revision, provider conversation,
start time and deadline. Duplicate IDs within a connection are refused without another
broker call or credential response; reconnects can reuse request IDs. A committed `resolving` or
`response_unknown` operation blocks later requests after a controller crash;
credentials and prompts are never replayed to resolve uncertainty.

The existing broker force-refresh deadline remains eight seconds. A refresh
operation permits a response only before its nine-second deadline; the response
write itself has at most 500 milliseconds. At most three refresh operations may
start within sixty seconds, including successful credential responses that are
followed by another unauthorized request. Failed resolutions have a one-second
backoff. Each new receipt records its connection scope. The control loop marks
that connection closed on return or task cancellation through a blocking worker
with a bounded record-lock wait.

Terminal `credentials_supplied` and `refresh_failed` receipts from proven closed
connections are pruned after sixty seconds, preserving the full retry window
and failure backoff across reconnects. Live-connection replay identities,
uncertain outcomes, unknown states and unscoped receipts without connection
lifetime evidence remain retained. A process crash or retirement-write failure
also retains receipts conservatively.

The store retains up to sixty-four unpruned operations per runtime and refuses
further operations at capacity rather than discarding protected identities. A new
runtime receives a fresh fenced store; in-place recovery keeps the runtime,
conversation, account and coordination claims unchanged.

Receipts distinguish `credentials_supplied`, `refresh_failed`, and
`response_unknown`; credential delivery alone does not prove provider health.
Each receipt retains a bounded snapshot of unknown in-flight operation selectors
and their total count, marks untracked effects unknown, and records
`replayed: false`. Existing authentication incident reporting remains best effort;
the recovery receipt must persist before broker or transport side effects.
Account failover and terminal outcomes follow the
[policy-gated authentication failover contract](specs/auth-failover-v1.md).

Missing/stale structured evidence permits an inferred fallback: at most eight
terminal lines and 4096 bytes, with a 250 ms sampling deadline. Only a bottom
provider UI error status immediately above an empty prompt (or the final status
line) is accepted: Claude's error-status marker plus a fixed login error, or
Codex's square error marker plus `unexpected status 401 Unauthorized`. Bare
assistant text and older scrollback are excluded. Persistent screen content
stays deduped until that status disappears. Fallback-only or unavailable
sampling is exposed as degraded detection health; unavailable or queued relay
delivery is exposed in the receipt. The target is observed failure to durable
owner inbox delivery within 60 seconds when sources and transport are available;
these degraded states do not establish that target or a measured p99.

### Authentication JSON fields

`auth_incident` is omitted when the current runtime has no incident or carried
quarantine schedule. When
present, list and board expose the same object; nullable members are present
with JSON `null`. Consumers must ignore additive unknown members.

| Field | Type | Meaning / values |
| --- | --- | --- |
| `incident_id` | string | SHA-256 hexadecimal incident identity. |
| `session_id` | string | Managed session identity. |
| `runtime_incarnation` | string | Original runtime launch identity. |
| `runtime_generation` | unsigned integer | Original runtime generation. |
| `provider` | string | `claude` or `codex`. |
| `account_nickname` | string or null | Configured account nickname, when available. |
| `observed_at` | timestamp string | Host observation time. |
| `source` | string | `claude_stop_failure`, `codex_app_server`, `codex_external_refresh`, or `terminal_pattern`. |
| `confidence` | string | `authoritative`, `observed`, or `inferred`. Terminal fallback is `inferred`. |
| `status` | string | `auth_failed`, `recovered`, `recovery_failed`, or `runtime_replaced`. |
| `recovery_result` | string or null | `healthy`, `credentials_refreshed`, `refresh_failed`, `recheck_exhausted`, or `runtime_replaced`; null before a final result. |
| `recovery_observed_at` | timestamp string or null | Final-result observation time. |
| `recovery_path` | string, optional | Fixed class of the settlement: `model_health` (a healthy provider turn was observed), `credential_recheck` (an operator or scheduled same-account recheck passed) or `controller` (an automatic refresh, failover or lifecycle result). It never carries an identity, credential or request body. A bound `account_nickname` is a binding, not evidence of healthy authentication. |
| `auto_settle` | object, optional | Bounded daemon same-account recheck schedule, runtime selectors, attempts and fixed refusal; see [scheduled recheck](specs/claude-auth-recovery-v1.md#scheduled-credential-recheck). |
| `notification` | object | Initial incident notification receipt, shaped below. |
| `recovery_notification` | object | Final-result notification receipt, with the same shape. |

Each notification receipt contains these four members, plus an optional
`escalation` member:

| Field | Type | Meaning / values |
| --- | --- | --- |
| `submission_started_at` | timestamp string or null | Start of a potentially committing submission; null before submission or after a proven missing-owner preflight failure. |
| `delivered_at` | timestamp string or null | Time durable inbox delivery was confirmed; does not imply the owner read or accepted the message. |
| `message_id` | string or null | Mailbox identity after submission is recorded. |
| `degraded_reason` | string or null | `owner_unavailable`, `relay_unavailable`, `relay_pending`, `delivery_unknown`, or `identity_conflict`; null when no degradation is recorded. |
| `escalation` | object, optional | Present only after an `owner_unavailable` attempt: `route` (`observation_spool`), `state` (`recorded` once the event was appended, `unavailable` while the spool cannot be written and the next 15-second tick retries) and `recorded_at` (timestamp or null). `delivery_unavailable` means the independent deliverable record could not be retained and will be retried. Spool recording never implies delivery; see [alert delivery](specs/auth-alert-delivery-v1.md). |

Timestamps use RFC 3339 strings. `auth_detection_health` is omitted before
observation and otherwise contains exactly one of `structured`,
`degraded_terminal_fallback`, `degraded_source_unavailable`,
`degraded_store_unavailable`, or `degraded_queue_capacity`. The board omits an
unrecognized health value or an incident that fails typed deserialization.
`runtime_replaced` results remain in private history because their original
runtime is omitted from the current projection.

## Conversation transitions

`agent-session clear <id> [--expect-idle] --format json` admits only a verified
idle interactive Codex or Claude Code session. A native clear changes the
provider conversation while retaining the managed session and runtime generation.
The owning adapter records the exact new resume identity and replaces the old
turn snapshot with an authoritative `waiting` state without a current or last
turn. The record and activity mutation share the lifecycle/activity fences;
persistence failure restores the previous snapshot and retains the private native
observation for `agent-session rebind <id> --format json` recovery.

Codex observes successful primary TUI `thread/start` responses before forwarding
them, refreshes its attached marker and control-thread reducer, and excludes
system-ephemeral threads. Clear/rebind require the live proxy’s versioned
conversation capability, not only the historical app-server runtime kind. Older
running proxies must stop/resume through the upgraded installation before
conversation mutation. Claude accepts only the primary `SessionStart` with
`source: "clear"` through the paired hook helper. That private receipt contains
only the provider identity and event/source. The paired hook retains runtime-bound
private recovery evidence before invoking the helper, so helper failures do not
lose the identity. It never enters public hook output
or the activity journal. Compaction and foreign-runtime receipts are excluded.

Recovery never chooses a thread by transcript recency. Codex requires an
unambiguous loaded primary identity and live provider `idle` status under the
turn-admission fence. A rejected manual-marker mismatch retains the live TUI
thread candidate for this verification, even with two loaded primary threads.
Claude requires a newly observed clear receipt and verified idle activity; a
matching new idle projection can complete an interrupted record commit. A prior
runtime receipt is ignored after resume. Recovery refuses a stale same-ID Claude
receipt and does not infer missing native evidence from history.
A clear confirmation timeout returns `conversation-clear-unconfirmed`, retaining
state for inspection/rebind instead of claiming that tmux delivery proved clear.

## Normalized turn event

The local-only command accepts one JSON object on stdin:

```text
agent-session activity event <session-id> --stdin --format json
```

Required fields:

| Field | Contract |
| --- | --- |
| `schema_version` | exactly `agent-session.turn-event.v1` |
| `event_id` | opaque id used for idempotency |
| `runtime_id` | exact active `AGENT_SESSION_RUNTIME_ID` |
| `provider` | `codex`, `claude`, or `dsh` must match the session; `dsh` is admitted only for the profile-backed pane rule below |
| `kind` | `turn_started`, `attention_requested`, `attention_cleared`, `progress`, `stop_observed`, `turn_completed`, or `turn_failed` |
| `confidence` | `authoritative`, `observed`, or `inferred` |

Optional allowlisted fields are `provider_session_id`, `provider_turn_id`,
`failure_reason`, `attention_id`, `attention_kind`, `source_kind`, and
`provider_time`, optional `completion_candidate_reason`, optional
`completion_candidate`, and optional `tool_call`. These completion fields are
admitted only on observed Claude provider-hook `stop_observed` events and contains no raw hook
payload. `tool_call` is admitted only on Claude provider-hook events and is
defined with the approval rules below. Unknown
keys fail parsing. Identifiers are bounded, non-empty, and control-free.
`attention_requested` requires an opaque correlation id and one of `approval`,
`clarification`, `authentication`, or `other`; `attention_cleared` requires the
matching id.

`completion_candidate_reason` is a content-free Claude Stop classification:
`candidate`, `stop_hook_active`, `stop_hook_active_missing`,
`stop_hook_active_invalid`, `background_tasks_present`,
`background_tasks_missing`, or `background_tasks_invalid`. Only `candidate`
may accompany `completion_candidate: true`. Missing fields do not prove the
absence of continuing hooks or background work. New producers and consumers
must be upgraded together before using this optional event field.

When completion evidence remains pending for 300 seconds after the latest Stop
or provider progress, every harness projects `phase: unknown`, inferred
confidence, and diagnostic reason `completion_evidence_missing` in read views.
The diagnostic includes the bounded quiet threshold, evidence timestamp, signal
name, and optional candidate reason. Pending attention takes precedence.
Reads leave activity snapshots, journals, revision, and completion history
unchanged; control admission and lease renewal retain the stored working state.
Fresh progress removes this uncertainty, and an admitted completion still
produces normal waiting state. This rule does not infer completion or attention.

`failure_reason` is valid only on an authoritative `turn_failed` event and is
limited to `usage_exhausted`, `authentication`, `organization`, `billing`,
`invalid_request`, `service`, `max_output_tokens`, or `unknown`. Claude Code's
documented structured `StopFailure.error` is reduced into that allowlist. Raw
error details, rendered provider errors, transcript paths, prompts, and
assistant content are discarded. A raw Codex interactive notification does not
carry an equivalent structured failure field. An agent-session-managed Codex
app-server v2 runtime reuses the stable v1 `source_kind: "provider_hook"` wire
value only after its live bound thread/turn reports terminal `failed` plus exact
`usageLimitExceeded`. The protocol-owned v2 admission also accepts an exact
terminal non-retrying `serverOverloaded` failure as `provider_capacity` without
adding that reason to the public v1 ingestion union. Within v1, `provider_hook` denotes authoritative
provider-structured evidence from either a hook or the bound protocol; that
metadata-only projection can authoritatively arm usage-reset auto-resume. For
an opted-in managed Codex runtime, the internal capacity projection instead
arms a bounded daemon-owned continuation; rendered terminal prose never does.

Provider hooks and direct `activity event` callers may supply bounded raw opaque
session or turn identifiers. Ingestion projects them to runtime-scoped SHA-256
opaque values before storage or exposure; an already projected `local:v1:` value
must carry exactly 64 hexadecimal digest characters. When exact provider resume
identity is known it must match; when it is not known, the first non-empty
projected provider session id binds the runtime; later changes or identity-less
events are rejected.

A `dsh` event is admitted for a tmux pane only when the persisted session has
`agent: dsh`, a server-owned runtime launch profile, and an absolute persisted
agent binary. Profile-less, relative-launcher, and every cross-provider pair
remain provider mismatches. Provider identifiers use the admitted event
provider's runtime-scoped projection domain. Native external DSH records continue to take
turn evidence only from their plugin-owned liveness sidecar. A DSH provider-hook
event for the exact active runtime generation succeeds only when that sidecar
already proves a live turn; the result projects the sidecar state and does not
mutate the activity document. Stale generations, missing or invalid sidecars,
and every other provider remain fail-closed.
For a profile-backed pane, `activity hook --agent dsh` maps the DSH bridge's
`pre_llm_call` and `post_llm_call` callbacks to observed start and authoritative
completion; DSH has no approval callbacks.

The host receive time is canonical. Provider time is accepted only as inert
metadata in v1 and never advances state ahead of host observation. Runtime id
and provider mismatch are rejected before timestamping, journaling, or reducing.

For the existing Codex tmux/TUI integration, the provider appends one bounded
JSON argument to this owned command:

```text
agent-session activity notify --agent codex <payload>
```

Only `type == "agent-turn-complete"` is recognized. Both `thread-id` and
`turn-id` are required and projected through the same runtime-scoped namespace
as hook identifiers. A matching notification emits authoritative
`turn_completed`; raw `Stop` remains `stop_observed`. Fields such as `cwd`,
`input-messages`, and `last-assistant-message` are discarded before the
normalized event is built. Unknown notification types no-op; invalid, oversized,
stale, or mismatched payloads fail open to Codex and cannot complete the turn.

## Codex completion recovery and quiet projection

If a start hook is missed, accepted progress can name a newer turn while the
stored current-turn ID remains older. A bound authoritative completion can
recover that ID only from the latest retained provider progress, in the same
runtime and conversation. Attention, known older turns and newer work prevent
recovery; a bare completion or Stop cannot supply the missing binding.

Existing stuck snapshots recover on their first view read, including after
daemon startup, when a retained completion still matches the latest progress.
Without such proof or pending Stop completion evidence, a Codex working view
remains `working` with inferred confidence after 300 seconds without provider
progress. The
`codex_progress_quiet` diagnostic reports `current_turn.last_progress_at` as
the signal; snapshots lacking it use the observed turn start. Reads preserve
stored provider history and use a stable quiet deadline.

List, board and stream expose the raw progress timestamp and this additive
diagnostic for a quiet projection:

```json
{"reason":"codex_progress_quiet","quiet_threshold_seconds":300,"quiet_since":"2026-10-13T12:00:00Z","quiet_signal":"current_turn.last_progress_at"}
```

Quiet is a possibly-idle scheduling hint, not completion or input authority.
Native notification candidates must recheck the exact bound conversation's
idle status, without reading turns, before input admission. Busy or unavailable
providers stay queued; terminal recipients with unknown phase keep their existing refusal.
This includes a resumed Codex runtime in `starting` with a retained last turn.
When the native probe verifies the bound conversation idle, an unchanged,
healthy startup with no open turn, attention, or pending evidence becomes
authoritative `waiting`. The preceding last turn is preserved; no completion
event is invented. New activity during the probe prevents this transition.
There is no time-based startup fallback.
Guarded terminal wake integration remains owned by #2432.

Controller admission and active-lease renewal omit the quiet presentation
heuristic, so long tools are not denied or expired because of missing progress.
Runtime health and actual completion checks remain enforced.

## Turn state

Example:

```json
{
  "schema_version": "agent-session.turn-state.v1",
  "phase": "needs_input",
  "phase_changed_at": "2026-07-10T12:31:28Z",
  "revision": 8,
  "source": {
    "kind": "provider_hook",
    "provider": "codex",
    "confidence": "observed"
  },
  "semantic_event": {
    "kind": "progress",
    "observed_at": "2026-07-10T12:31:41Z"
  },
  "current_turn": {
    "provider_turn_id": "turn-id",
    "started_at": "2026-07-10T12:31:08Z",
    "last_progress_at": "2026-07-10T12:31:41Z",
    "attention": {
      "kind": "approval",
      "requested_at": "2026-07-10T12:31:28Z",
      "pending_count": 2,
      "certainty": "conservative"
    }
  },
  "last_turn": {
    "provider_turn_id": "previous-turn-id",
    "started_at": "2026-07-10T12:24:31Z",
    "completed_at": "2026-07-10T12:28:04Z",
    "outcome": "completed"
  }
}
```

Phases are `starting`, `working`, `waiting`, `needs_input`, `unresponsive`, and
`unknown`. `unresponsive` is an observed terminal projection after guarded
submit or clear keys leave a recognized idle pane unchanged past the configured
bound. It clears on the next observed pane change. Busy turns and unobservable
panes are excluded; see [composer response observation](specs/stale-drafts-v1.md#unresponsive-pane-observation).
Source kinds are `provider_hook`, `console_observation`,
`terminal_heuristic`, and `runtime`. Last-turn outcomes are `completed`,
`interrupted`, `failed`, `operator_reconciled`, and `unknown`.

`pending_count` is the only client-visible attention correlation summary. Only
runtime-scoped projections of provider session/turn identifiers may be exposed.
Provider request ids and the active runtime id remain protected in local
activity storage. At most 64 attention correlations are retained in the
snapshot; additional requests contribute only to a bounded overflow summary
and keep `needs_input` conservatively latched until a new turn or completion.

`current_turn.last_progress_at` is optional additive v1 metadata. It advances
monotonically only when accepted provider-hook evidence proves progress for the
active runtime and open turn. It never uses terminal output, spinner text,
focus, browser clocks, or provider-supplied timestamps. Exact
`AskUserQuestion` completion/failure counts as progress while clearing only its
own runtime-scoped clarification correlation. Old snapshots omit the field and
remain valid.

`semantic_event` is the last accepted structured provider event and contains
only its allowlisted kind plus daemon receive time. Polling, SSE delivery,
reconnect, and browser clocks do not advance it. Clients may use its age to
describe uncertainty, but age never proves Waiting or completion.

Attention `certainty` is `exact` only when the selected provider adapter owns a
stable request/resolution correlation. A Claude approval bound to its tool call
by [tool-call correlation](#approval-and-tool-call-correlation) is `exact`.
Generic permission and identifier-less elicitation evidence is `conservative`.
Old snapshots omit the field and therefore deserialize conservatively.

Optional `diagnostic.reason` values are producer-owned allowlisted codes:
`completion_evidence_pending`, `attention_authority_mismatch`,
`provider_projection_unavailable`, `runtime_activity_unhealthy`,
`activity_state_unavailable`, `interrupted_suspected`, `codex_progress_quiet`,
`codex_startup_idle_unverified`, and `completion_evidence_missing`. Quiet diagnostics include only bounded
`quiet_threshold_seconds`, `quiet_since`, and `quiet_signal` fields described
above, plus the optional allowlisted `completion_candidate_reason`. They never
prove completion, clear attention, or authorize input.
Free-form provider or runtime errors never cross the session-view or stream
boundary.

A managed Codex runtime still in `starting` after 30 seconds without verified
primary-thread idle evidence projects `unknown` with inferred confidence and
`diagnostic.reason: codex_startup_idle_unverified`. This read-only condition is
also retained by board and activity-stream projections. It preserves the
startup timestamp, revision and durable history; a later provider idle
observation may still settle startup. Age never proves readiness.

Optional `shadow_observation` is ordinarily diagnostics-only. It contains
`observer_version`, a bounded `rule_id`, daemon `observed_at`, one of
`working`, `needs_input`, `waiting`, or `unknown`, and a `disagrees` flag. It
never confirms completion, clears attention, or authorizes automation.

### Claude structured state source

For a running Claude session the shadow sampler reads the provider's own
state first and uses screen rules only when it is unavailable. The source is
`claude agents --json` of the installed `claude` binary (resolved like other
provider binaries, never from a session record). Provider facts are pinned to
the research-preview agent view; see
[Session runtime v1](specs/session-runtime-v1.md).

- **Supported versions.** Two entry shapes are read. The `status` shape
  (`idle`, `busy`, `waiting`, plus `waitingFor`) was captured from Claude Code
  2.1.317. The `state` shape (`working`, `blocked`, `done`, `failed`,
  `stopped`) is read from the provider's current documentation of
  `claude agents --json` ([agent view](https://code.claude.com/docs/en/agent-view));
  it lists `state` for background sessions only and documents no minimum
  version; interactive entries captured from 2.1.317 carry `status` and no
  `state`. The `state` fixtures are synthetic. A version that
  reports neither field value set reads as `unknown_state` and falls through.
- **Call.** One call per provider config directory serves every session
  sampled within a 2-second window; sessions sharing a directory never spawn
  per session. Each config directory has its own registry, so a session with
  its own `CLAUDE_CONFIG_DIR` (launch profile or bound account) is read from
  that directory. The call is bounded to 1.5 seconds and 1 MiB.
- **Environment.** The child environment is built explicitly: `HOME`, `PATH`,
  `TMPDIR` and the session's `CLAUDE_CONFIG_DIR` only. API-key style
  credentials and endpoint overrides are never inherited.
- **Match.** An entry matches a session by its provider session id
  (`provider_resume.session_id`) equal to the entry `sessionId`. Duplicate
  entries for one id resolve only when they agree; conflicting ones are
  unavailable, never decided by list order.
- **Mapping.** `state` decides when the entry has one, and `status` then only
  says whether the process is busy; `status` decides only when `state` is
  absent. `state: working` projects `working`; `blocked` projects
  `needs_input`; `done`, `failed` and `stopped` project `waiting` (not
  working). Without `state`, `busy` projects `working`, `idle` projects
  `waiting` and `waiting` projects `needs_input`.
  `waitingFor: "permission prompt"` is recorded as
  `claude_agents_waiting_permission_prompt` and any other or missing reason as
  `claude_agents_waiting_other`; provider text is never copied. The rule id
  does not say which field decided.
- **Evidence.** The observation has `observer_version: claude-agents.v1` and a
  `rule_id` of `claude_agents_busy`, `claude_agents_idle`,
  `claude_agents_waiting_permission_prompt` or `claude_agents_waiting_other`.
  The `disagrees` flag compares the projection with the hook-derived phase. An
  idle reading over an open turn is sustained like the screen interrupt
  marker below, without the screen marker's composer check.
- **Fallthrough.** When the binary is missing, the call times out or fails,
  the output is malformed, the session has no provider session id, no entry
  matches, entries for it conflict, the `state` is present but not one of
  the five values above (its `status` is then not consulted), or the status
  of an entry without `state` is not `idle`, `busy` or `waiting`, the screen
  rules decide. The resulting observation (`observer_version: terminal-shadow.v1`)
  carries `claude_agents_unavailable` with one of `binary_missing`, `timeout`,
  `failed`, `malformed`, `no_provider_session`, `unmatched`,
  `conflicting_entries` or `unknown_state`. The activity stream carries `rule_id` and
  `observer_version` only; the reason appears in session views.

The `waiting` and `waitingFor` fixtures follow the provider spike and were
not live captured; only the `idle` and `busy` entries match output captured
from the installed CLI. The `state` fixtures are synthetic and follow the
provider documentation.

Screen rules stay in place as the fallback; removing them is tracked
separately.

For Claude and Codex, two consecutive samples of the provider interrupt marker,
at least 15 seconds apart, may project `phase: unknown` with
`diagnostic.reason: interrupted_suspected` and inferred terminal-heuristic
provenance. This is uncertainty, not confirmed `interrupted` or `waiting`: the
open turn and last-turn outcome remain intact.

Runtime launch/generation and activity revision fence both samples; a new
prompt or any newer hook invalidates them. Claude samples the current viewport
rather than scrollback, and a nonempty prompt row after its latest interrupt
marker invalidates that marker. An earlier turn's marker cannot establish
idleness for a fresh turn. Claude requires an empty idle composer; its drafts
cannot produce this projection.

Codex uses the `Conversation interrupted` marker; plain composer
text cannot distinguish a placeholder from a draft and supplies no completion
or waiting evidence. Claude also recognizes a Running tool status below the
latest interrupt marker as Working evidence; an earlier tool status cannot mask
a newer interrupt. A working indicator, attention, missing marker, stale sample,
or runtime replacement cannot produce this projection. A ready serve
activity broker refreshes on a 15-second timer even without
hooks or HTTP polling, and completed shadow writes refresh stream snapshots.

Sample timestamp changes alone update the cache without broadcasting a new
snapshot; changed rules or turn-state projections still reach subscribers.
Sampling is throttled to once per 15 seconds for each runtime, including across
activity revisions, giving a nominal bound of 30 seconds plus collection and
stream debounce latency. One-shot CLI views read only the cache. No observer or
serve collector means this signal is unavailable. Claude has no interrupt hook
and [Stop excludes user interrupts](https://code.claude.com/docs/en/hooks#stop).

## Attention correlation authority

The v1 reducer is provider-neutral. Exact provider adapters may request and
clear attention only with the same opaque, runtime-scoped correlation. A
different id and an uncorrelated `progress` event never prove resolution;
completion, failure, a new turn, or a new runtime remain the only boundaries
that may clear all outstanding attention.

Each runtime selects one attention authority when it is created or resumed and
keeps that authority for the lifetime of the runtime. Hook and protocol
observations are not paired by arrival time, event kind, or semantic similarity.
A provider protocol may be selected as the exact authority only when the
admitted interaction matrix proves complete request coverage and the runtime
suppresses the corresponding generic attention hook at its source. Otherwise
the runtime selects conservative hook authority. An event from a suppressed
attention source is an authority-invariant breach: it neither creates attention
nor advances `last_progress_at`, and the runtime degrades to unknown until a new
runtime or resume selects authority again.

Client dismissal remains presentation-only fingerprint suppression. It cannot
clear producer-owned attention or influence authority selection.

For Codex, a raw or unmanaged runtime selects `hook`; its generic
`PermissionRequest` remains a conservative approval latch. An audited managed
app-server runtime selects `protocol` and injects
`AGENT_SESSION_ATTENTION_AUTHORITY=protocol`. The guarded direct command
suppresses its generic hook before invoking the helper, including when that
helper predates authority-aware ingest.

Protocol authority is unavailable until either the guarded direct command or
one owned matcherless `agent-hook dispatch --product codex` permission group
is verified, with no second direct reporter. Dispatcher ownership also requires
converged `agent-hook doctor` evidence. Normalized dispatcher permission events
use the same immutable authority disposition as raw hooks: protocol is a
content-free no-op, hook authority retains conservative approval, and a mismatch
marks the runtime unhealthy.

Advisory dispatch can still exit successfully with
a warning; that does not undo the unhealthy state. The dispatcher and activity
helper must use the validated release pair. App-server transport may still run
with hook authority. This source-guard prerequisite does not audit a new
provider version.

Its private proxy admits only the audited blocking
request method allowlist and `serverRequest/resolved`. Request ids retain their
JSON `string` versus `int64` type only in the bounded in-memory pending table;
each admitted request occurrence receives a fresh opaque correlation token.
The raw request id is never persisted or exposed, and a provider may reuse the
same id after its prior request resolves without hitting durable replay
deduplication. A recognized malformed request, wrong-turn request, observation
loss, malformed proxy data, projection failure, or hook/record authority
mismatch writes a durable runtime-generation unhealthy marker. A private health
fence linearizes the scoped pending poison marker against activity commits and
the durable auto-resume submission claim; stable activity mirroring then uses
the session-record lock. The marker owns a stable degradation revision and
phase timestamp. Invalid, unreadable, or parseable-but-nondegraded marker states
fail closed instead of being exposed.
The public v1 state becomes `unknown`, auto-resume becomes unavailable, and
later same-runtime events are rejected; only a new runtime generation can
remove the marker, select authority, and recover. If an open turn has no
provider turn id, its first non-null exact attention request binds the turn;
later mismatches fail closed. Claude is the exception: it runs one turn at a
time and names it by `prompt_id`, but a turn whose `UserPromptSubmit` was never
recorded (a lost or refused hook delivery) is never announced, so a Claude
attention request for another turn interrupts the stale open turn and opens the
requesting one instead of being refused. A request for the most recently closed
turn is duplicate metadata only while a different identified turn is open. In
every other case it is live and is reduced as `needs_input`: with no turn open
it opens that turn again, and an open turn with no provider turn id is bound to
it. Claude keeps stamping hooks with a prompt `idle_prompt` already closed
while a background subagent still works under it. Nullable MCP elicitation
remains admitted without inventing a turn id.

For Claude Code, `AskUserQuestion` remains exact through `tool_use_id`.
`Elicitation` and `ElicitationResult` are also exact when both carry the same
non-empty `elicitation_id`: form mode maps to `clarification`, and URL mode maps
to `authentication`. Since Claude's hook contract makes the id optional, a
request without it is a conservative latch and a result without it is a no-op.
Generic permission and notification signals remain conservative. The managed
Claude setup excludes the uncorrelated `permission_prompt` notification because
current Claude versions emit it as a duplicate of `PermissionRequest`, and an
`AskUserQuestion` `PermissionRequest` is ignored because the same interaction is
already owned by exact PreToolUse/PostToolUse correlation.

Managed Claude setup also installs a general `PreToolUse` hook that normalizes
to uncorrelated `progress`: a continued turn last observed at `idle_prompt` can
re-establish `working` as soon as a tool starts. The exact `AskUserQuestion` arm
is evaluated first, so its `PreToolUse` remains `attention_requested` rather
than progress. `SubagentStop` is deliberately neither installed nor admitted as
progress because it identifies a completed subagent without correlating that
callback to active parent work; a late background callback must not resurrect a
genuinely waiting parent turn. Positive progress never clears pending attention.

Completion candidates use Claude `Stop` with explicit `stop_hook_active: false`
and an empty `background_tasks` array.

- After one second without a newer accepted event, session views project observed
  `waiting` and a completed last turn, including when the composer contains a draft.
- Missing or malformed fields, active stop hooks, and background tasks remain
  non-final. `StopFailure` remains immediately terminal and authoritative.
- Reads use a deterministic deadline and do not write activity. The next accepted
  event materializes the settled state before reduction. Any event inside the
  window cancels the candidate; later continuation progress reopens `working`.
- `idle_prompt` is late observed confirmation, not the completion dependency. The
  confidence stays observed because another concurrent hook can continue after
  the quiet window.
- The support floor is Claude Code 2.1.206; the hooks contract was checked through
  2.1.289 against the [official reference](https://code.claude.com/docs/en/hooks#stop).

The one-second window remains because Stop hook data cannot prove that the turn
ended. Claude runs every matching `Stop` hook in parallel and any of them may
continue the turn, and the payload (`stop_hook_active`, `background_tasks`,
`session_crons`, `last_assistant_message`, `prompt_id`) says nothing about the
other hooks. A continuing hook's next `Stop` carries `stop_hook_active: true`,
which is non-final, so completing at once would project `waiting` while Claude
works until a late `idle_prompt`. Immediate completion is therefore limited to
payloads that prove it: `StopFailure` and `idle_prompt`.

The structured source that removes the window is the `claude agents --json` idle state
([#3590](https://github.com/sympoies/nils-cli/issues/3590)); the screen rules
go after both land ([#3592](https://github.com/sympoies/nils-cli/issues/3592)).
No settings reader is added to prove that no other Stop hook exists: plugin,
`--settings` and skill-frontmatter hooks are not enumerable from the hook.

The coordination notification controller applies a narrower input-safety rule:
an exact-runtime `Stop` may authorize only the fixed body-free mailbox prompt
after a short debounce with no later accepted provider hook, no pending attention,
and no attached tmux client. Any later accepted provider event cancels that
notification-only waiting signal.

## Deterministic transition rules

| Input | Rule |
| --- | --- |
| new runtime | interrupt an open turn, preserve it as last turn, clear attention, enter authoritative `starting` |
| primary Claude `SessionStart` with `source: "startup"` or `"resume"` | only from `starting` with no open turn, attention, or pending evidence, in the same runtime and bound provider conversation (raw or runtime-scoped projected id, as `agent-hook` forwards it with `source` regardless of model settings): enter authoritative provider-hook `waiting` |
| primary Codex `thread/start` response reports `thread.status.type: "idle"` | after the proxy binds the exact primary thread and before forwarding the response: settle healthy, unchanged `starting` to authoritative provider-hook `waiting`, with no fabricated turn or completion; absent or active status does not settle |
| native Codex mailbox probe observes the exact bound conversation idle | only from healthy `starting` with unchanged runtime and activity revision and no open turn, attention, or pending evidence: enter authoritative provider-hook `waiting`, preserving the preceding last turn |
| `turn_started` | interrupt an older open turn, clear old attention, enter `working` |
| `attention_requested` | keep current start time, add one opaque pending request, enter `needs_input`; a Claude request for another turn first interrupts the stale open turn |
| correlated `attention_cleared` | remove only that request, advance monotonic `last_progress_at`, and remain `needs_input` while any remain |
| uncorrelated `progress` | advance monotonic `last_progress_at`; may establish/retain `working`, but never clears attention |
| Claude `progress` with `tool_call` `finished` for a call bound to an approval | as `progress`, and remove only that approval ([tool-call correlation](#approval-and-tool-call-correlation)); any other pending attention remains |
| `stop_observed` | increment evidence revision and journal it; never changes to Waiting |
| admitted server-operator reconciliation | close only the exact stop-observed open provider turn as `operator_reconciled`, enter authoritative `waiting`, and retain typed reconciliation provenance |
| matching `turn_completed` | close current turn, clear attention, enter `waiting`; authoritative Codex notifications require the exact open turn id |
| Claude idle after qualifying exact current Stop with a changed prompt ID | close the stopped turn in the same bound runtime and provider conversation only with no newer accepted work, pending attention, or known old-turn ID |
| matching `turn_failed` | close current turn with failed outcome, clear attention, enter `waiting` |
| unapplied completion for another turn | journal receipt and retain accepted revision, source, phase, Stop diagnostic, and pending completion candidate |
| duplicate exact-replay `event_id` | no state or revision change within the sliding active-runtime replay window (at least the last 4096 exact events); uncorrelated Claude progress instead uses the short semantic guard |
| missing/prior runtime id | reject before host timestamp or reducer |
| corrupt snapshot | expose safe `unknown`; list/serve/delete remain available |
| unhealthy authority/projection | expose `unknown`, accept no later event in the same runtime, recover only on a new runtime generation |

Claude `PermissionRequest` signals for tools other than `AskUserQuestion` mean
that a permission dialog is actually being shown, so they emit
`attention_requested` even when the payload reports `permission_mode:
"bypassPermissions"`. The mode hint does not override the observed prompt;
bypass mode retains a root/home deletion circuit breaker. The request names the
tool, carries the same `tool_input` as the `PreToolUse` that preceded it, and
has no `tool_use_id` or request id (Claude Code 2.1.317, measured). It is
therefore correlated to its call as described below, and a request that cannot
be correlated stays on the conservative latch. A same-turn `Stop` clears known
approval latches without clearing clarification or ambiguous overflow. Missing
or mismatched prompt identity remains latched. `StopFailure`, completion, new
turn, and runtime boundaries clear all attention.

### Approval and tool-call correlation

The optional event member `tool_call` carries evidence without raw content for Claude
`PreToolUse`, `PermissionRequest`, `PostToolUse`, `PostToolUseFailure` and
`PermissionDenied` events other than `AskUserQuestion`, which stays exact
through its own `tool_use_id`. Its fields are `phase` (`started`, `permission`
or `finished`), `call_id` (a runtime-scoped projection of `tool_use_id`, on
`started` and `finished`), `input_digest` (a runtime-scoped projection of the
tool name and `tool_input`, on `started` and `permission`) and `tool_name` (a
bounded name over `[A-Za-z0-9_.:-]`, omitted otherwise). The tool input, its
description and the raw call id never leave the hook. Missing or malformed
optional fields yield no `tool_call` and never fail the event.

The reducer keeps the open calls of the current turn (calls with a `started`
and no `finished` yet, at most 64, scoped by the projected turn id) and applies
these rules:

| Evidence | Rule |
| --- | --- |
| `started` | record the call; a later `started` for the same call id replaces it |
| `permission` | name the tool on the pending approval; bind it to the call when exactly one unbound open call of the same turn has the same `input_digest`, and mark that approval `certainty: exact` |
| `permission`, zero or several matching calls | no binding; the approval stays `conservative` (identical parallel calls cannot be told apart) |
| `finished` for a call with a bound approval | remove that approval only; other pending approvals remain, and the phase returns to `working` when none remain |
| `finished` for another turn, an unknown call or an unbound approval | no effect on attention |

The public `attention` view gains the optional `tool_name` of the first pending
approval, for example `{"kind":"approval","pending_count":1,"certainty":"exact",
"tool_name":"Bash"}`. It never exposes the input, the digest or any id.

Limits:

- Correlation needs the `PreToolUse` and the result hooks registered for the
  tool. Managed setup registers hooks per tool matcher; a tool without them has
  no recorded call, so its approval stays `conservative` and `tool_name` is
  still recorded from the request.
- There is no approval-resolved hook in the
  [official contract](https://code.claude.com/docs/en/hooks#permissionrequest).
  `PermissionDenied` fires only on auto-mode denial. A manual dialog denial and
  an interrupt emit no hook at all (measured on Claude Code 2.1.317: no
  `PermissionDenied`, `PostToolUseFailure` or `Stop` followed), so the recorded
  wait ends only at the next `UserPromptSubmit`, a same-turn `Stop`,
  `StopFailure`, completion or a runtime boundary. The structured source that
  can observe denial is the `claude agents --json` state
  ([#3590](https://github.com/sympoies/nils-cli/issues/3590)).
- Two open calls with the same tool and input are ambiguous by construction;
  the approval is not bound and a result for either call does not clear it.
- A result clears the approval when the tool runs after approval, including
  when the tool itself then fails.
- A dropped hook delivery is invisible. If the `PreToolUse` of the call that
  asks is lost while an identical call is open, that open call is the only
  candidate: the request binds to it as `exact` and its result clears the
  approval early. No payload field shows a drop, so this cannot be detected;
  the case needs a lost hook and an identical open call.
- When the 64-call cap evicts the oldest open call, the approval bound to it
  becomes `conservative`, because that call can no longer deliver the result
  that clears it.
- No input summary is recorded. The redaction rules exclude tool input, so the
  approval carries the tool name and nothing else about the call.
- `input_digest` is SHA-256 over the runtime id, tool name and full input with
  no secret. It is stored only in the private activity document and journal
  and is never public, but a short, guessable input can be confirmed offline
  by a reader of those files; the evidence holds no raw content, not no
  derivative.
- An `agent-hook` that emits `tool_call` requires an `agent-session` that
  accepts it; no compatibility guard exists. An older `agent-session` rejects
  the unknown event key (`activity-event-invalid`). The hook then reports
  `session-activity-failed`, which only warns for read-only, advisory or off
  operations and makes other operations, including mutating Claude tool calls,
  block under enforce coordination. Install both in one step on each host,
  `agent-session` first when the installer orders them.

User-owned or previously configured `permission_prompt` notification reporters
normalize to the same conservative latch, but the managed setup does not
install that duplicate source.

Revision is monotonic for each accepted non-duplicate event and runtime
boundary. Phase timestamps change only when the phase changes. Durations are
derived by clients and are never persisted separately.

A raw Stop also projects `diagnostic.reason:
completion_evidence_pending` while the turn remains open. The next accepted
provider event replaces that diagnostic; it does not retroactively treat Stop
as completion. An unapplied completion is receipt evidence only: it does not
replace the accepted semantic event, diagnostic, provider event selectors, or
pending Stop candidate.

Claude's observed `idle_prompt` can carry a new prompt ID after a local
`/compact` without a new `UserPromptSubmit`. Reconciliation uses the latest
accepted qualifying Stop candidate for the exact current turn in the same runtime
and bound provider conversation. An active, background-task, or reentrant Stop
cannot authorize this exception. It requires working state, no later accepted
work or attention, no pending attention, and an idle ID absent from both the
known previous turn and correlated turn metadata in the retained journal unless
the receipt is explicitly marked `applied: false`. A retained terminal receipt
without the admission member conservatively denies this exception. Journal
read uncertainty denies only this changed-ID exception. IDs outside the bounded
retention window are treated as unseen.

It completes the stopped turn under its original ID and journals the raw idle
ID separately. No command or conversation content is collected or inferred.

## Missing authoritative completion reconciliation

The HTTP server exposes
`POST /sessions/{id}/activity/provider-turn/operator-reconcile/v1` as one
narrow Bearer-only operator repair for a provider turn whose latest exact
provider event is `stop_observed` but whose authoritative completion signal is
missing. It is not another completion detector and never accepts provider
input or target-session capability authority.

Success advances only the activity revision, closes the exact current turn
with outcome `operator_reconciled`, and enters authoritative `waiting`. Typed
reconciliation provenance is owned by that matching `last_turn`; a later turn
or runtime activation cannot expose it as unrelated top-level state, and an
id-less completion never inherits it without exact same-turn identity. Provider
completion, replacement runtime, queued/newer provider evidence, or attention
wins before admission and leaves the activity document unchanged.

For snapshots created before the latest-provider turn selector was persisted,
the operator path may recover only that absent selector from a strictly parsed,
bounded, exact-matching final provider-hook journal entry. It never overrides a
present selector, skips malformed records, or accepts a mismatched/newer tail.
The complete
request/result schemas, fence ordering, admission and preservation rules,
receipt contract, idempotency, and stable failures are normative in
[Session Coordination V1](specs/session-coordination-v1.md#operator-provider-turn-reconciliation).

## Client presentation projection

Durable phase remains conservative for old-client safety. A new client may
derive simultaneous work plus attention from the additive timestamps without
rewriting server state:

| Durable evidence | Presentation |
| --- | --- |
| open turn, no pending attention | Working from `started_at` |
| attention pending, no later provider progress | Needs input from `requested_at` |
| attention pending and `last_progress_at > requested_at` | Working plus input requested, keeping both timers |
| exact clarification clear removes the final request | Working from the original `started_at` |
| proven completion or failure | Waiting from `completed_at` |

Unavailable, stopped, resumable, and connecting health/runtime states remain
higher priority than this activity projection. Progress never implies that an
uncorrelated permission or notification request was answered.

## Persistence and concurrency

Each session owns:

- `activity.json`: atomic mode-0600 snapshot;
- `activity.journal.jsonl`: atomic mode-0600 metadata journal, bounded to 256
  events and 64 KiB;
- `activity.replay.bin` and `activity.replay.1.bin`: the two tables of the
  replay window, each a fixed-size mode-0600 open-addressed index for 4096
  runtime-scoped event-id digests in the unchanged `agent-session-r1` format,
  with a versioned launch-id/generation header;
- `.activity.lock`: mode-0600 cross-process advisory lock.

Activity files are separate from `session.json`, so title/resume writes and hook
writes cannot overwrite each other. Every reducer transaction holds the lock,
validates both the active launch id and runtime generation, records one pending
journal entry in the atomic snapshot, updates the fixed replay index when the
event requires exact replay protection, appends the bounded journal
idempotently, and clears the pending marker. A later event or runtime transition
repairs an interrupted split write before reduction. New journal entries record
the optional `applied` boolean; `false` receipts never authorize recovery retries.
Entries without this member require matching accepted terminal state,
selectors, source, and timestamps before retrying recovery. Duplicate recovery
requires retained admission evidence even when the longer replay index still
deduplicates the receipt. Unapplied late receipts do not become known accepted
turn history. The replay index is
separate from the shorter journal retention, gives expected O(1) duplicate
checks without growing the JSON snapshot, and is a sliding window: the exact
event with zero-based index `i` in a runtime generation (the snapshot's
`seen_event_count` before it) goes to table `(i / 4096) % 2`, and the first
event of each window resets that table's file first. Table 1 is consulted only
once the count has passed the first window. Exact dedupe therefore covers at
least the last 4096 and at most 8192 exact events, ingest never stops for
capacity, and recovery never needs a restart, resume, or new session id
(sympoies/nils-cli#1962). A window's table is reset before the count that
selects it becomes durable, so a crash there leaves the count unchanged; the
reset is redone idempotently when a pending boundary insert is repaired. The
state never needs a new runtime generation to recover. Only table 0 must match
for views; a missing, stale, or wrongly sized table 1 is re-initialized for the
current runtime on the next exact event, and a table found full because a
window's clear was skipped (for example a boundary repaired by a binary without
the window) is reset for the current window. Either self-heal loses at most one
window of dedupe.

Mixed versions: hook binaries and serve runtimes upgrade independently, so
table 0 keeps the exact pre-window file name, format, and header, and there is
no migration. A binary that predates the window still validates and reads table
0, so its views stay valid; past 4096 exact events its own ingest still refuses
with `activity-dedupe-capacity-reached` until it is upgraded, and it ignores
table 1. An upgraded binary continues from a full table 0 by starting table 1.
Uncorrelated Claude provider-hook `progress` has idempotent reducer
semantics and no stable provider event id, so it keeps bounded journal and
split-write repair coverage but relies on the short semantic replay guard rather
than consuming exact replay slots. The replay file header must match the
snapshot runtime tuple. A missing, truncated, or swapped index for a nonempty
exact-replay horizon fails closed and exposes Unknown; creating the index also
syncs its parent directory. Provider-hook events additionally use a short
metadata-only semantic replay guard so concurrent duplicate delivery cannot
interrupt the same turn, inflate an uncorrelated attention latch, or rewrite an
already observed completion.
Unknown additive JSON fields survive supported reads and writes. A corrupt or
future-version snapshot is moved to a private quarantine file before a fresh
runtime snapshot is written. Session deletion removes the entire session
directory.

## Privacy and provider adapter boundary

The schemas forbid prompt/assistant/terminal/transcript text, commands, tool
arguments/results, paths from provider transcripts, credentials, tokens, and
free-form provider errors. Raw hook payloads are parsed in memory and projected
onto the allowlist; the Codex notification adapter applies the same boundary to
the provider's single JSON argv. Content fields are never printed or serialized
by agent-session. Codex supplies that JSON as a process argument, so prompt,
assistant, and cwd content remains transiently observable through same-host
process inspection until the helper exits. Restricted process visibility is a
deployment requirement; eliminating this upstream argv exposure requires a
future provider-supported stdin/metadata-only transport or App Server boundary.

Provider hook and notification normalization lives behind the
`activity/provider.rs` adapter boundary. The central module retains the typed
reducer, persistence, replay, and public projection.

The `activity/shadow.rs` observer samples only running Claude or Codex sessions
whose structured evidence is unknown, at least five minutes old, or has been
missing for at least five minutes, plus open Claude or Codex working turns for
the interrupt-uncertainty rule. The long-lived serve collector schedules
sampling in detached bounded workers and immediately returns cached metadata;
one-shot CLI views only read that cache and never start work that could be lost
at process exit. Sampling runs outside the activity-ingestion lock, uses a
process-wide concurrency cap of four, and is cached for 15 seconds. Each tmux
metadata/capture command is bounded to 250 ms and 16 KiB, and only the
metadata-only observation sidecar is persisted. Pane titles and capture bytes
are discarded in memory. Each sidecar is fenced by runtime launch identity and
generation, with the active record revalidated immediately before persistence.

`provider-prompt.v1` is a separate, advisory attach/title protocol. It is not a
turn event source, it is not persisted into activity files, and a prompt-event
drop/reconnect/title timeout cannot change durable turn state.

## Setup and diagnostics

```text
agent-session activity setup --agent <provider> --dry-run
agent-session activity setup --agent <provider> --apply
agent-session activity setup --agent <provider> --repair
agent-session activity setup --agent codex --repair --dry-run
agent-session activity setup --agent codex --repair --expected-preview-digest sha256:<reviewed-plan-digest>
agent-session activity setup --agent <provider> --remove
agent-session activity doctor [--agent <provider>] --format json
```

`activity setup` always forwards to the shared `agent-hook setup` owner, using
the shared toolchain selector: an executable `AGENT_SESSION_BIN` selects its
same-directory companion, otherwise approved absolute roots are searched. It
never searches `PATH` or the working directory. It maps the compatibility provider and digest options, requests the versioned
JSON contract, and adds `compatibility_owner: "agent-hook"` to the returned
result. It never writes provider configuration itself.

If `agent-hook` is absent or cannot be started, setup returns the typed
`agent-hook-setup-unavailable` error with shared unavailable exit `69` and
install-and-repeat-preview guidance. A valid child error envelope preserves
the shared `1`, `64`, `65`, `69`, or `70` exit class returned by `agent-hook`;
malformed or unsupported child output remains a data-contract failure.
There is no embedded registration fallback, including for `--apply`,
`--repair`, or `--remove`, so a mixed-version installation cannot reactivate a
second writer.

`activity doctor` remains a read-only compatibility diagnostic. Because
`agent-hook` owns provider registration, Codex readiness requires one bounded,
strictly typed `agent-hook doctor` result for Codex. The result must be
supported and `converged`, report the exact expected owned count, report zero
retired residue, and carry valid configuration and policy digests. Missing,
failed, malformed, oversized, multi-record, or provider-mismatched evidence
fails closed. Claude readiness uses bounded `agent-hook setup --product claude
--dry-run` evidence, validates configuration and policy digests, and checks
exact owned dispatch event groups. Its optional `missing_owned_events` array
lists missing dispatch event names and is omitted when empty. `config_path`
reports the checked settings file, resolved from `CLAUDE_CONFIG_DIR` or
`CODEX_HOME` when set and otherwise from the provider default.

In this diagnostic, `configured` describes agent-hook registration readiness.
`classification` remains the provider activity support class; Claude's
`partial` value does not indicate a configuration fault. The existing
`can_launch_worker` field remains the conjunction of `classification ==
"supported"` and `configured`, so it is not a general check that a provider
session can start. The diagnostic separately recognizes exact
pre-dispatch `agent-session` hook and Codex notify shapes, including a bounded
audited Computer Use outer wrapper whose exact helper path is a regular
executable with no symlink below the active config root. It reports conflicts
without provider content, probes provider versions with bounded timeouts, and
selects the newest current-runtime diagnostic deterministically. The retained
`activity hook` and `activity notify` commands continue to ingest already
installed compatibility callbacks fail-open while provider registration
converges on `agent-hook`.
