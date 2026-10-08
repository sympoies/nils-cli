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
`provider_time`. Unknown
keys fail parsing. Identifiers are bounded, non-empty, and control-free.
`attention_requested` requires an opaque correlation id and one of `approval`,
`clarification`, `authentication`, or `other`; `attention_cleared` requires the
matching id.

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

Phases are `starting`, `working`, `waiting`, `needs_input`, and `unknown`.
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
stable request/resolution correlation. Generic permission and identifier-less
elicitation evidence is `conservative`. Old snapshots omit the field and
therefore deserialize conservatively.

Optional `diagnostic.reason` values are producer-owned allowlisted codes:
`completion_evidence_pending`, `attention_authority_mismatch`,
`provider_projection_unavailable`, `runtime_activity_unhealthy`,
`activity_state_unavailable`, and `interrupted_suspected`. Free-form provider or
runtime errors never cross the session-view or stream boundary.

Optional `shadow_observation` is ordinarily diagnostics-only. It contains
`observer_version`, a bounded `rule_id`, daemon `observed_at`, one of
`working`, `needs_input`, `waiting`, or `unknown`, and a `disagrees` flag. It
never confirms completion, clears attention, or authorizes automation.

For Claude and Codex, two consecutive samples of the provider interrupt marker,
at least 15 seconds apart, may project `phase: unknown` with
`diagnostic.reason: interrupted_suspected` and inferred terminal-heuristic
provenance. This is uncertainty, not confirmed `interrupted` or `waiting`: the
open turn and last-turn outcome remain intact. Runtime launch/generation and
activity revision fence both samples; a new prompt or any newer hook invalidates
them. Claude requires an empty idle composer; its drafts cannot produce this
projection. Codex uses the `Conversation interrupted` marker; plain composer
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
app-server runtime selects `protocol`, injects
`AGENT_SESSION_ATTENTION_AUTHORITY=protocol`, and suppresses that generic hook
before the helper is invoked, including when the installed helper predates
authority-aware ingest. Protocol authority is unavailable until that guarded
installed command is verified and no second direct unguarded reporter is
present; app-server transport may still run with hook authority. Its private
proxy admits only the audited blocking
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

Raw Claude `Stop` remains journal evidence and does not change the public
`TurnPhase` by itself. The coordination notification controller applies a
narrower input-safety rule: an exact-runtime `Stop` may authorize only the
fixed body-free mailbox prompt after a short debounce with no later provider
hook, no pending attention, and no attached tmux client. Any later provider
event cancels that notification-only waiting signal.

## Deterministic transition rules

| Input | Rule |
| --- | --- |
| new runtime | interrupt an open turn, preserve it as last turn, clear attention, enter authoritative `starting` |
| `turn_started` | interrupt an older open turn, clear old attention, enter `working` |
| `attention_requested` | keep current start time, add one opaque pending request, enter `needs_input`; a Claude request for another turn first interrupts the stale open turn |
| correlated `attention_cleared` | remove only that request, advance monotonic `last_progress_at`, and remain `needs_input` while any remain |
| uncorrelated `progress` | advance monotonic `last_progress_at`; may establish/retain `working`, but never clears attention |
| `stop_observed` | increment evidence revision and journal it; never changes to Waiting |
| admitted server-operator reconciliation | close only the exact stop-observed open provider turn as `operator_reconciled`, enter authoritative `waiting`, and retain typed reconciliation provenance |
| matching `turn_completed` | close current turn, clear attention, enter `waiting`; authoritative Codex notifications require the exact open turn id |
| matching `turn_failed` | close current turn with failed outcome, clear attention, enter `waiting` |
| late completion for older turn | retain the newer current phase |
| duplicate exact-replay `event_id` | no state or revision change within the sliding active-runtime replay window (at least the last 4096 exact events); uncorrelated Claude progress instead uses the short semantic guard |
| missing/prior runtime id | reject before host timestamp or reducer |
| corrupt snapshot | expose safe `unknown`; list/serve/delete remain available |
| unhealthy authority/projection | expose `unknown`, accept no later event in the same runtime, recover only on a new runtime generation |

Claude `PermissionRequest` signals for tools other than `AskUserQuestion` mean
that a permission dialog is actually being shown, so they emit
`attention_requested` even when the payload reports `permission_mode:
"bypassPermissions"`. The mode hint does not override the observed prompt;
bypass mode retains a root/home deletion circuit breaker. Because these
approvals have no correlated clear event, they keep the conservative latch
above until completion, a new turn, or a runtime boundary. User-owned or
previously configured `permission_prompt` notification reporters normalize the
same way, but the managed setup does not install that duplicate source.

Revision is monotonic for each accepted non-duplicate event and runtime
boundary. Phase timestamps change only when the phase changes. Durations are
derived by clients and are never persisted separately.

A raw Stop also projects `diagnostic.reason:
completion_evidence_pending` while the turn remains open. The next accepted
provider event replaces that diagnostic; it does not retroactively treat Stop
as completion.

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
repairs an interrupted split write before reduction. The replay index is
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
`AGENT_HOOK_BIN` when explicitly set and otherwise resolving `agent-hook` on
`PATH`. It maps the compatibility provider and digest options, requests the versioned
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
`agent-hook` is the provider-registration owner, Codex launch readiness
requires one bounded, strictly typed `agent-hook doctor` result for Codex.
The result must be supported and `converged`, report its exact expected owned
count, report zero retired residue, and carry valid configuration and policy
digests. Missing, failed, malformed, oversized, multi-record, or
provider-mismatched evidence fails closed. The diagnostic separately recognizes exact
pre-dispatch `agent-session` hook and Codex notify shapes, including a bounded
audited Computer Use outer wrapper whose exact helper path is a regular
executable with no symlink below the active config root. It reports conflicts
without provider content, probes provider versions with bounded timeouts, and
selects the newest current-runtime diagnostic deterministically. The retained
`activity hook` and `activity notify` commands continue to ingest already
installed compatibility callbacks fail-open while provider registration
converges on `agent-hook`.
