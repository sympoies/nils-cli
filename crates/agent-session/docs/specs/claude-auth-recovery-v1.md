# Claude same-account auth recovery v1

The serve daemon reconciles current-runtime Claude authentication incidents on
its existing 15-second observation loop. Managed session and provider
conversation identities remain stable; a successful stop/resume advances the
runtime incarnation and generation. Account failover follows the separate
[policy-gated failover contract](auth-failover-v1.md).

## Prompt quarantine

A committed current-runtime `auth_failed` incident fences new Claude prompts.
The activity authentication failure also fences input if the bounded incident
store cannot admit another incident. CLI sends, structured HTTP prompts, raw
terminal input, automatic continuations and mailbox notifications share this
admission rule. `allow_blocked`, bare Enter and printable keys cannot bypass it.
Control keys that do not submit text remain available for interruption.
Unavailable incident storage fails closed for prompt input.

Operator `restart` remains available for a quarantined runtime, with or without
`--force`. After the existing runtime identity, idle/empty-composer checks and
input fence, only the restart controller's fixed `/exit` command bypasses prompt
quarantine. A sealed internal command authorizes that single lifecycle input;
CLI, HTTP, raw terminal and mailbox input cannot construct it. Sending `/exit`
through those input paths remains quarantined. `--force` still overrides only
idle quiescence; runtime identity and verified-stop requirements remain in force.

HTTP quarantine returns `409 auth-quarantined`. Keyed prompt-v2 attempts retain
a `refused` receipt with admission proof (`pasted: false`, `submitted: false`);
repeating the same request key returns that receipt without dispatch.

The refusal details carry a fixed, metadata-only `reason` (`auth_failed`) and
`exit` list (`restart`, `switch_account`); they never include credential or path
data. A Claude `StopFailure` that carries `agent_id` fired inside a subagent, so
it does not end the main turn and is ignored: a subagent-only authentication
failure neither opens an incident nor quarantines the session. The main thread
never carries `agent_id`.

Quarantine does not replay or cancel provider tool effects. A later successful
provider turn can settle an incident through the existing health observation.
An incident belonging to a replaced runtime does not quarantine an unrelated
incarnation. A controller-owned replacement remains quarantined until its
readiness check completes; a failed readiness check leaves it quarantined.

## Operator recheck

No turn can be admitted while quarantined. A repaired credential can settle
through the daemon schedule below or an explicit same-account recheck. `account switch` to the account the session is already bound to
queues nothing; on a quarantined session it re-runs the broker credential
preflight and, when the failed launch's publication is known, requires a newer
publication for that account. Only then it settles the owning incidents as
`credentials_refreshed` and admits input again. A refused preflight returns
`claude-account-recheck-refused` and leaves every receipt unchanged. A recovery or
failover operation still in flight is never overridden (`recovery_in_progress`),
and a later authentication failure quarantines the runtime again.

## Scheduled credential recheck

On the existing 15-second observation loop, the daemon schedules same-account
credential rechecks at +30 seconds, +2 minutes and +10 minutes from the
incident's observation. Each tick runs at most one due attempt. A delayed daemon
may run a due attempt on its next tick; it never launches a separate timer.
The recheck uses the same credential preflight and publication proof as the
operator recheck. It does not switch accounts, restart the runtime, submit input
or change the failover policy gate. Existing controller-owned recovery is never
overridden; `recovery_in_progress` is a recorded refusal.

The session-record lock covers runtime/account validation, credential preflight
and settlement. Any next-account intent, including an intent on the same runtime,
cancels this schedule with `next_account_queued`; a changed or missing applied
account cancels it with `account_changed`. Account selection and queued intent
remain unchanged. A stale daemon runtime snapshot is refused with
`runtime_changed`. A successful recheck records `credentials_refreshed` and
`recovery_path: credential_recheck`, then stops. A later authentication failure
starts a new incident and schedule. A model-health settlement also stops probes.

`auth_incident.auto_settle` exposes the scheduled runtime incarnation/generation,
`started_at_epoch`, state (`scheduled`, `settled`, `cancelled`, `exhausted`), fixed
`refusal` or null, and up to three attempts with `attempted_at_epoch` and a fixed
`result` (`started`, `credentials_refreshed`, `recovery_in_progress`,
`credentials_not_republished`, `credential_preflight_refused`, or
`recheck_unavailable`). Broker details are never copied into this result.
An attempt is committed as `started` before credential work; a daemon
crash consumes that attempt, so restarting the daemon cannot repeat it without
bound. After the last refusal (or an interrupted last attempt), the quarantine
remains and `recovery_result: recheck_exhausted` triggers the existing final-result
notification and owner-unavailable escalation. Recording an alert is distinct
from delivery.

If an incident still fences an in-place replacement runtime on the same account,
the daemon applies a fresh schedule from that runtime's `started_at`, retaining
the original incident identity and failed-publication proof. Its projection stays
visible with the schedule's current runtime selectors. Rebuilding the daemon
without replacing the runtime preserves the consumed budget. An unrelated
replacement that is no longer quarantined gets no schedule.

## Publication prerequisite

At account binding and each launch, the broker's `list --provider claude`
response must advertise `capabilities: ["credential_state_v1"]` for generation
tracking. The consumer then reads `credential-state --provider claude --account
<nickname> --format json`. Each invocation has a two-second deadline and uses
the existing bounded broker process runner.

The response schema is `agent-session.credential-state.v1`. Recovery requires
matching provider/account, host identity, authority version and snapshot, plus
a currently `published` completion receipt. The target, authority view and
`last_published` receipt must agree on version and snapshot; policy revisions
and publication timestamps must agree. All required stores must be verified,
including account credentials, account configuration and directory preparation.
Partial, pending, missing and blocked publications never authorize a restart.
Revoked, expired and invalid authority observations do not authorize it either.

The token-free launch checkpoint records account, host, authority UUID,
authority epoch UUID, decimal generation and snapshot UUID, bound to that
runtime incarnation. A replacement must have a strictly higher numeric
generation and a different snapshot in the same account, host, authority and
epoch. Binding revisions, timestamps, successful materialization and an old
`last_published` receipt do not substitute for that evidence. Unknown broker
fields are not copied into checkpoints or recovery receipts.

Compatibility launches remain supported without this capability. A runtime
without a launch checkpoint reports `launch_generation_missing` on auth loss;
it cannot safely infer which generation failed. Missing advertised capability
reports `capability_missing`. Both remain quarantined. Enabling production
publication and launching tracked runtimes are deployment-owner duties.

## Bounded recovery and crash behavior

The optional `auth_incident.same_account_recovery` object uses schema
`agent-session.claude-auth-recovery.v1` and records one durable
operation UUID, original conversation, baseline/replacement publications,
deadline, poll count, state, fixed reason, replacement runtime selectors and
unknown-effect metadata. It is retained with the original incident; it is not
an independent public history endpoint. Current-runtime incident projections
retain their existing visibility rules.

The controller commits identity and a 120-second publication deadline before
broker or lifecycle work. It admits at most eight publication polls. Temporary
broker failures wait within that budget; expiry reports `publication_deadline`.
An account intent, changed conversation, invalid canonical resume metadata or
superseded runtime blocks the operation. It never selects another account.

Claude currently exposes no supported credential reread operation, so the
receipt records `reread: "unsupported"` and proceeds to controlled stop once a
new publication is proven. No prompt is sent to test credential adoption.

The controller revalidates publication, acquires the existing restart input
fence, and calls the lifecycle stop without waiting for idle or typing an exit
command. The existing exact-runtime and platform stop proofs remain mandatory.
It records unknown operation selectors after verified stop, retires the old
coordination incarnation and claims, and resumes using only the canonical
stored provider conversation arguments. The replacement launch must capture
the selected generation or a newer publication in the same identity domain.
Existing lifecycle substep deadlines apply, followed by a ten-second readiness
budget. `runtime_resumed` proves readiness, not a healthy model turn.

Stages are `waiting_generation`, `stopping`, `stopped`, `resuming`,
`awaiting_ready`, `runtime_resumed`, `recovered_in_place` and terminal `blocked`.
An observed healthy turn while waiting settles `recovered_in_place` and cancels
the replacement. A per-session private
operation lock and the session record lock serialize effects. Persisted stop
stages can re-enter the stopped-proof path. A persisted replacement identity
can reconcile readiness after a controller crash. A `resuming` attempt with
only the old durable runtime identity reports `resume_outcome_unknown`; it
never blindly launches again. Operator lifecycle reconciliation owns that
ambiguous case.

`unknown_operation_count` counts nonterminal leases and abandoned leases without
an effect outcome observed after stop. This includes leases already retired by
the provider's exit wrapper.
`unknown_in_flight_effects` lists at most eight lease/claim selectors with
`state: "unknown"`; a larger count explicitly indicates omitted selectors.
Tool arguments, targets, execution capabilities and outcomes are excluded.
`untracked_effects: "unknown"` covers provider effects without registered
leases, and `replayed: false` records that the controller sent no continuation
prompt and replayed no tool request. Existing lease retirement records them as
abandoned without claiming their external effects failed or completed.
