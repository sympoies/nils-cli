# Policy-gated authentication failover v1

After bounded same-account recovery, the serve authentication watchdog can
admit one account switch. Claude must first exhaust its published-generation
wait; missing launch publication or broker generation capability also permits
candidate preflight. Other lifecycle, identity or uncertain-resume refusals
never authorize failover. Codex gives the official refresh path 120 seconds
from the incident; unresolved refresh or response receipts block switching.
The controller never refreshes tokens by inventing a provider request.

## Broker policy and capacity

Both providers require the broker's existing list envelope to advertise
`capabilities: ["auth_failover_v1"]` and
`selection_strategies: ["next_with_capacity"]`. Capacity selection alone does
not grant auth failover authority. Brokers that omit either advertise no
automatic auth-switch policy and produce `policy_capability_missing`.

The controller invokes the existing selector with an auth-specific reason:

```text
select [--provider claude] --strategy next_with_capacity --after <failed-account>
  --exclude <failed-account> --reason authentication --format json
  --session-id <managed-session> --runtime-incarnation <runtime> --operation-id <operation>
```

The response uses the provider's normal envelope, an `account` nickname (or
`null` when no candidate exists), and this token-free proof:

```json
{
  "auth_failover": {
    "policy_allowed": true,
    "capacity_confirmed": true,
    "checked_at_epoch": 1900000000,
    "expires_at_epoch": 1900000010
  }
}
```

The broker owns account allowlists and auth-switch policy. It must obtain
fresh network-confirmed capacity for the returned account and may set
`policy_allowed: true` only when configured policy authorizes this session's
auth recovery. The selection must belong to the freshly listed accounts and
must differ from the failed account. A proof cannot come from the future,
be older than ten seconds, be expired, or live longer than ten seconds.
List and select each have a two-second process deadline.

Claude rechecks proof freshness after preparing the target account directory,
before stopping. Codex repeats policy/capacity selection at the queued intent's
actual apply boundary, including binding-repair drains; the exact target must
still be selected. The operation UUID also identifies that queued intent.
Unrelated explicit account intents retain their existing behavior.

## Fencing, results and quarantine

The optional `auth_incident.failover` receipt has schema
`agent-session.auth-failover.v1`. It records one operation UUID, original
account and binding revision, provider conversation, target nickname, proof,
30-second switch deadline, replacement runtime selectors and unknown-effect
selectors. Arbitrary broker fields and credentials are never copied into it.
States are `candidate_preflight`, `switching`, `account_switched`,
`credentials_refreshed`, `healthy`
and terminal `blocked_no_credential`. Fixed reasons distinguish no candidate,
policy refusal, stale capacity, changed identity, deadlines and unknown outcomes.

Receipt stages are committed before broker and switch effects. A crash in
preflight is terminal; it does not silently rerun selection. A persisted Codex
intent can complete through the existing account-switch drain, which checks
fresh policy before applying it. A missing or replaced intent, ambiguous
Claude launch, or superseded runtime never causes a repeated switch.
The switch deadline stops pending admission; an already applied binding can be
reconciled without replaying its effect.

Codex switches accounts through the existing explicit queued account path and
keeps its conversation and runtime incarnation; its existing claims remain
associated with that incarnation. Claude uses the existing verified stop,
coordination retirement and canonical resume paths without waiting for idle.
It retains managed session and provider conversation IDs, advances runtime
generation/incarnation, and retires the stopped runtime's claims. Readiness
does not claim a healthy model turn.

Input remains quarantined while switching or blocked, including a controller
replacement that failed readiness. The receipt records unknown operation
selectors and `untracked_effects: "unknown"`, with `replayed: false`. No
continuation prompt, interrupted tool operation or provider request is replayed.
An applied switch permits input; only an observed healthy model turn ends the
failure chain. Another auth fault before that turn produces
`consecutive_fault` without selecting another account.

The official same-account refresh path remains available after a refusal
when no account intent is pending. A successful credential response releases
input quarantine as `credentials_refreshed`; it still does not prove model
health. Damaged incident reporting must not disable that official retry path.
Queued and applying intents refuse refresh. Failed or malformed intents do
not prevent refresh of the current bound account; their evidence remains
intact and continues to fence new prompts until applied or cancelled.

A later successful explicit Codex binding supersedes the prior automatic
refusal's input quarantine. Official refresh and health observations also
settle older receipts for the same provider conversation, even when the
observation belongs to a newer incident. Historical unknown-effect metadata
remains intact; these observations never replay an operation.

Recovery-result notifications use the existing owner outbox. Failover result
keys include operation and result identity so a previous same-account result
cannot conflict with the terminal failover result.

## Rollback

Auth incident and Codex refresh receipt readers validate schemas strictly.
Earlier builds can reject stores containing new fields with
`auth-incident-store-unavailable` or refresh `store-invalid`. In particular,
the connection-retention fields added to `codex-auth-recovery.json` are not
readable by builds preceding that change.

Before downgrading, stop the affected runtime through the verified lifecycle
path and preserve its receipt files for investigation. Move the affected
`auth-incidents.json` and/or `codex-auth-recovery.json` out of the session
directory only after stop is proven, then downgrade and resume with a fresh
runtime incarnation. Configure the archive destination outside runtime state.
Never clear a live store to bypass quarantine, retry limits, unknown-effect
fences or a policy denial. Archived unknown effects still require reconciliation;
rollback does not make them safe to replay.

Broker capability enablement, live-provider acceptance and deployment are
separate operations; source delivery does not enable a broker's policy.
