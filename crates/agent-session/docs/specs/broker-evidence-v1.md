# Broker evidence v1

## Purpose and compatibility

The ordinary recovery consumer and producer agree on additive, authenticated,
non-executing evidence. `prepare-admission-proof` and `proof` never call admission,
replay a receipt, drain completion events, renew claims, reclaim operations, or
change the registry. Exact admission replay remains compatible for older
consumers and producers. Retired orchestration and worker lifecycle are outside
this contract.

Released provenance: v1.36.31 has no proof commands and supports exact admission
replay plus complete/reconcile. These commands are additive after that baseline;
this specification does not claim a release or deployment. Consumers feature
probe both command help surfaces before choosing the proof path. A post-release
consumer acceptance run verifies that path separately.

## Preparation

```text
agent-session broker prepare-admission-proof [--session ID] --claim UUID --claim-revision N --operation KIND --targets-file JSON --execution-token-file FILE --admit-idempotency-key KEY --capability-file FILE --format json
```

An omitted session uses `AGENT_SESSION_ID`. The capability defaults to the
existing private runtime capability file. Authentication requires the ordinary
owner's exact current, ready, heartbeat-fresh broker incarnation. A scoped worker
credential does not grant this owner evidence surface.

The result data has schema
`agent-session.broker-admission-proof-preparation.v1` and a `selector` with
exactly five fields:

```json
{
  "kind": "admission",
  "admit_idempotency_key": "admission-key-0001",
  "claim_id": "claim-selector",
  "claim_revision": 1,
  "expected_request_digest": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}
```

The digest uses the released admission canonicalization: claim ID/revision,
operation, canonical filesystem/provider/pull-request targets, checkout bindings,
descendant identity and execution-token digest. The key is a separate receipt
namespace selector. No token, raw targets, checkout path or capability is returned.
Preparation does not prove admission, validate current scope coverage, or grant
execution authority. It can prepare an old exact request after claim release.
Persist the selector and private replay material before submission.

## Bounded batch proof

```text
agent-session broker proof --session ID --expected-incarnation ID --expected-generation N --batch-file JSON --capability-file FILE --format json
```

The batch must be an owned private regular file, with no final symlink or hard
links, at most 64 KiB. Its schema is `agent-session.broker-proof-batch.v1` and
`selectors` contains 1 through 128 items. Each item is either the preparation
selector above or:

```json
{
  "kind": "operation",
  "lease_id": "lease-selector",
  "revision_floor": 1,
  "claim_id": "claim-selector",
  "claim_revision": 1
}
```

Authentication, incarnation and generation fences apply to the whole batch under
the exact session authority and observational registry locks. Generation must
match both the broker and persisted runtime. Stale/unauthorized requests fail
without evidence or registry mutation. Claim/lease mismatches, revision floors,
digest conflicts and malformed selectors fail only their indexed item. The output
never echoes malformed input.

Result data has schema `agent-session.broker-proof.v1`, `session_id`,
`session_incarnation`, `generation` and ordered `proofs`. Each item contains its
zero-based `index`, `ok`, and either `proof` or a bounded `error` with `code` and
`message`.

A found operation proof contains `kind: operation`, `found: true`, `lease_id`,
`revision`, `claim: {claim_id, revision}`, `state`, `outcome` (nullable), and
`provenance`. A retained admission adds `kind: admission`, `status: committed`,
`retained: true` and `provenance: retained_operation`, with the same exact lease,
claim, revision, state and nullable outcome.

| Evidence | Meaning for recovery |
| --- | --- |
| Retained `active`, no outcome | The committed admission may restore only this issued lease and revision. |
| `completing` or `reconcile_pending` | Retain private evidence; no executable admission is established. |
| Exact terminal state (`completed`, `failed`, `abandoned`, backward-compatible `expired`) | Retire only its unchanged matching private record. |
| Admission receipt and exact terminal complete/reconcile receipt, operation no longer retained | `status: committed`, `provenance: historical_receipt`, `retained: false`; `lease_id` and exact claim remain, while `state`, `revision`, `outcome` are null. This proves only terminality. |
| Missing or expired admission receipt, or admission receipt alone naming an absent operation | `status: unknown`, `provenance: not_retained`, `retained: false`. No execution or terminal authority. |
| Missing operation and exact terminal receipt | `found: false`, `provenance: not_retained`. No authority. |

Expired active safety TTLs and queued completion events project as `completing`
until mutation-owned maintenance processes them. Inspection itself never asserts
completion. Historical operation evidence requires a retained, unexpired terminal
complete/reconcile receipt with the exact principal, incarnation, lease,
claim/revision and revision floor. A historical admission receipt alone cannot
prove a removed lease terminal.

Consumers must validate the envelope fences and every item's index and binding.
A batch containing one terminal lease and another active or conflicting lease
retires only the terminal record. It must preserve all unrelated private records
and claims. Unknown evidence and absent fields never grant execution authority.
