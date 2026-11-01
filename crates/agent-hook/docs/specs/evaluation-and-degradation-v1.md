# Policy evaluation and degradation

Part of the [agent-hook v1 contract](agent-hook-v1.md).

## Deterministic evaluation

Rules select by product, event, and matcher, then sort by `(priority asc,
rule_id asc)`. A duplicate rule ID is invalid. Decision precedence is
`block > transform-conflict > transform > warn/context > allow`.

Multiple context values concatenate in rule order with a 16 KiB aggregate
limit. Provider-native rendering uses that complete aggregate even when the
first context rule supplied an otherwise reusable native envelope. Codex and
Claude context never lists allow or context reason codes. A `context` decision
without text renders the neutral `{}`. A `warn` decision without text names
only the `warn` reason codes that fired and omits an unknown or underived
semantic conflict, which records absent evidence rather than an overlap; when
nothing remains it renders `{}`. Service JSON and traces keep every reason, and
`block` reasons are unchanged. Identical
replacements coalesce. Different replacements are an explicit
`transform-conflict` block; transforms never compose implicitly. Failure
posture is typed per rule (`open`, `warn`, or `closed`), while locked privacy,
writer, transaction, and recovery rules must be `closed`.

An executable capability is judged by its exit status and its output, never by
how much of its stdin it chose to read. A capability that exits before draining
the delivered payload closes the read end, and the runner's remaining write
returns `EPIPE`; that is the child's decision and is not a capability failure.
The runner returns promptly with the child's own result instead of blocking to
the timeout, and a capability that stops reading and then fails still takes its
rule's failure posture. Scoring the broken pipe as a failure made a `closed`
plus `locked` rule deny at random, because whether the child won the race
depended on machine load, and a `locked` rule offers no override to recover
with.

`agent-session.activity.v1` is metadata observation, not mutation authority.
Failure to record a `Stop` observation returns the stable warning
`activity-stop-reconciliation-required` instead of denying provider
termination. This prevents a missing, stale, or temporarily unavailable
activity store from creating a non-terminating Stop-hook loop. A typed activity
fault also warns for conversation, a closed-set audited read-only `PreToolUse`,
and a request authenticated to durable `advisory` or `off` mode. Every other
non-terminal request retains the configured failure posture. No activity
warning releases or alters
claims, leases, operations, brokers, worktrees, or session state. Any retained
activity uncertainty therefore remains visible for typed external
reconciliation after the provider runner exits.

Every `Stop` warning renders
the provider-native neutral `{}` response and is never model-visible: Codex
`Stop` does not support additional context, and Claude turns `Stop` or
`SubagentStop` context into a new model turn, so a persisting warning would
re-prompt the model on every stop — the very loop this warning exists to
prevent (sympoies/nils-cli#1962). The warning never reaches the model; the
warning decision and its observability record remain for operators.

When the activity helper fails, the reason keeps the stable
`session-activity-failed` code and appends the helper's typed cause from its
`--format json` envelope, for example
`session-activity-failed: provider-turn-id-mismatch`. Only
`activity-*` and `provider-*` codes are surfaced; any other helper output is
dropped.

## Interaction lanes and degradation

A capability that cannot prove itself must not admit arbitrary mutation, and it
must also not take the whole session with it. Every canonical event belongs to one
lane, and a typed runtime fault is handled per lane:

| Lane | Events | Behavior on a typed runtime fault |
| --- | --- | --- |
| conversation | `UserPromptSubmit`, `SessionStart` | event-scoped degradation context with `coordination-degraded-read-only` |
| audited read-only | `PreToolUse` whose in-process effect is exactly `read_only` | warn with `activity-degraded-audited-read-only`; later authoritative rules still run |
| terminal | `Stop`, `StopFailure` | terminal warning with reconciliation-pending evidence |
| mutation | every other event | unchanged fail-closed posture |

Only faults with a known recovery path are degradable:
`coordination-unavailable`, `coordination-untrusted`, `coordination-invalid`,
`runtime-version-skew`, and `activity-helper-unresolvable`. An unrecognized error
keeps the existing fail-closed handling rather than being degraded on a guess.
Within `agent-session.activity.v1`, the same bounded handling additionally
recognizes its own helper and execution faults. An incomplete managed identity,
including selectorless identity while trusted managed metadata remains, is never
a degradation authority: it returns
`session-activity-identity-incomplete` and refuses provider work until the
trusted launcher establishes the exact session/runtime pair.

Bare `pwd` with no arguments or only `-L`/`-P`/`--logical`/`--physical` is the
initial audited Bash probe; composition, operands, or unknown options remain `unknown`. A trusted
durable `advisory` or `off` mode may warn for another activity fault, but this
does not change any other rule or admit an enforce-mode mutation.

A degraded decision carries the fault code first, then the stable lane code
(`coordination-degraded-read-only`, `activity-degraded-audited-read-only`, or
`activity-degraded-advisory-off`), so
diagnostics keep the precise cause. Its human context states one primary
diagnosis and one safe next action instead of a comma-separated list of every
selected policy; the full reason list remains available in `--format json` and on
the `agent-session.observation.v1` plane.

Degrading the conversation lane acquires no new authority: the prompt is admitted
as text, every mutation stays gated, and the original prompt is neither lost nor
executed more than once because the turn correlation digest bounds replay.

The conversation note applies to that event only. An authorized mutation may be
retried and is evaluated individually against its own tool/hook result; an old
conversation note is not a session-wide read-only restriction.

After conversation degradation, the next successful evaluation of the previously
degraded checks emits one `conversation-degradation-recovered` context. This
presentation state is private and bound to the exact product, managed session,
and runtime identity. Another identity, an unrelated successful rule, absent
rules or coordination state, a failed mutation, and terminal Stop handling do
not consume the pending observation. Subsequent healthy events and sessions that
never degraded emit no recovery note. Recovery reports those selected checks
only and grants no mutation authority; all other policy results stay effective.

The observation retains each affected selected rule and its capability identity,
including the configured reason. An unrelated coordination probe or a reused
rule ID with different capability settings cannot satisfy that observation.
Pending recovery is consumed only after fallible pre-emission trace work
succeeds; a trace failure leaves it available for the next successful event.

If persisting consumption fails, the original admission decision is emitted
without recovery context and pending recovery remains available. Presentation
state I/O never adds a denial.

The terminal lane has two degradation boundaries. A coordination transaction
that cannot run at all returns the compatibility classification
`coordination-stop-reconciliation-required`, but its diagnostic makes no
retained-state or gate claim and routes to read-only `agent-session broker
status`; without a warning boundary the first Stop delivery deadlocks before
provider re-entry metadata can help. Separately, when the provider reports Stop
re-entry and the aggregate still blocks, the decision becomes a warning:
re-entry proves the previous block changed nothing the gate awaits, so blocking
again only consumes the provider's consecutive-block budget.

Stop re-entry carries the typed coordination result into the terminal renderer;
the runtime handler supplies `runtime-kit.session-coordination-result.v1` with
one of `not-run`, `clean`, `pending`, or `unavailable`. Untyped provider payloads
are treated as unavailable, never as broker-state proof. The renderer does not
infer transaction state from generic provider actions or normalized reason
strings:

| Coordination result | Stable re-entry code | Disposition and recovery |
| --- | --- | --- |
| transaction explicitly reports a pending operation | `stop-reentry-reconciliation-pending` | `reconciliation-pending`; prescribe `agent-session broker reconcile` |
| transaction reports clean | `stop-reentry-terminal-exit` | `terminal-exit`; no recovery mutation |
| transaction did not run | `stop-reentry-terminal-exit` | `terminal-exit`; no coordination-state or gate claim |
| transaction result unavailable | `stop-reentry-terminal-exit` | `terminal-exit`; read-only `agent-session broker status`, with no retained-state or gate claim |

Every original reason is retained and no degradation releases or alters claims,
leases, operations, brokers, worktrees, or session state. A gate assertion is
made only for a reported pending operation in effective enforce mode; advisory,
off, skipped, and unavailable outcomes do not manufacture one.

## Release compatibility

A coordination registry written by a different release generation of the same
schema family is recoverable version drift, reported as `runtime-version-skew`
with a bounded `recovery_action` in its error details. A body that does not parse,
or that belongs to another schema family, remains `coordination-invalid`. Mutation
still fails closed on drift; the conversation lane degrades so the recovery can be
requested.

Broker records publish the release that created them. A minor or major difference
from the dispatching binary is recorded as `broker-release-skew` on the
observation plane with the peer release and its recovery action. A patch-level
difference and an unpublished release are not drift.
