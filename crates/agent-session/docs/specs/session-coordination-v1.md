# Session Coordination V1

## Status and ownership

- Status: implementation contract for `agent-session` coordination.
- Schema family: `agent-session.coordination.v1`.
- Owner: `nils-agent-session`.
- Compatibility: additive to `agent-session.session.v1`; clients that do not
  use coordination retain the current start, run, resume, list, glance, send,
  delete, activity, and serve contracts.

This specification defines privacy-preserving collision awareness for managed
agent sessions. Broker/session lifecycle automatically supplies presence and
checkout identity. Optional declared work context refines that presence; it is
never inferred from a prompt, transcript, log, glance, terminal bytes, or
assistant response. A mailbox is available only when metadata cannot resolve a
material uncertainty. Formal delegated implementation still uses the
provider-backed dispatch workflow.

## Threat and trust model

- Session IDs, incarnation IDs, claim IDs, message IDs, revisions, and public
  work-context fields are selectors and fences. They are not credentials.
- Every owner, sender, recipient, lease, and mailbox mutation is authenticated
  by a private per-incarnation capability created by the session broker.
- A capability is projected to the managed process through a 0600 file. CLI
  examples use `--capability-file`; the managed runtime may instead provide the
  trusted `AGENT_SESSION_CAPABILITY_FILE` path. Secrets are never accepted as
  public identifiers or emitted in argv, JSON, errors, logs, or provider data.
- Each broker incarnation also pre-creates one empty owner-only checkpoint file
  below that session's `0700` coordination directory and projects its exact
  path through `AGENT_SESSION_CHECKPOINT_FILE`. The filename binds the SHA-256
  digest of `AGENT_SESSION_RUNTIME_ID`; replacement removes the prior
  incarnation's file. This file is a private data-transfer boundary, not a
  credential, and does not relax checkpoint authentication or revision fences.
- Peer summaries and mailbox bodies are authenticated as peer-supplied data but
  remain untrusted. They cannot authorize commands, approvals, scope changes,
  credential access, or secret disclosure.
- The optional HTTP server has separate operator authentication. Knowing its
  bearer token does not manufacture a session capability, and knowing a
  session capability does not grant server-operator authority.

## Storage and locking

Session creation treats `<state-dir>` and `<state-dir>/sessions` as private trust ancestors. Each final
path component must be a current-user-owned directory and must not be a symlink. A newly created or
existing safe ancestor is opened without following its final component and tightened through that
directory handle to mode `0700` before any lifecycle-lock mutation. Session creation then opens or
creates `session-locks` relative to the pinned state root and opens the exact lock file with no symlink
following. The validated state-root, `sessions`, and leaf-session descriptors remain pinned through
record initialization and provider handoff.

Initial prompt, session, activity, coordination, and rollback mutations resolve through the pinned
descriptors; stable device/inode identity checks fence provider transport from a replaced pathname. The
same-user pathname-replacement limitation in [Operation
lease](coordination-schemas-v1.md#operation-lease) still applies at the provider boundary: this contract
prevents the CLI from redirecting its own storage mutations, but it is not an OS isolation boundary
around a later provider open. Hardening is deliberately non-recursive: existing sessions and unrelated
state remain unchanged.

Symlinked, foreign-owned, non-directory, or identity-changing ancestors fail before session or provider
side effects with `session-state-ancestor-untrusted`; unavailable metadata, creation, open, or permission
repair fails with `session-state-ancestor-unavailable`.

The private coordination root is `<state-dir>/coordination`, mode 0700. Regular
files containing coordination state or credentials are mode 0600. The root must
be owned by the current user, must not be a symlink, and must remain canonically
below the selected state directory. An untrusted owner, symlink escape, or
unrepairable permission drift fails before mutation.

One bounded registry lock serializes claim evaluation, claim acquisition,
operation leases, mailbox transitions, notification receipts, expiry, and
cleanup. Default lock timeout is 2 seconds. Lifecycle provisioning, revocation,
and notification admission checks retry transient lock contention within a
six-second monotonic budget, with capped backoff between attempts. TUI manual
input cancellation uses the same budget for transient session-record locks;
identity, account authorization and invalid state failures are not retried.
The launcher-owned broker exit hook retains its two-second lock wait so its
compatibility fallback fits the graceful termination deadline. No command
waits indefinitely.

Exhausted lifecycle contention preserves `coordination-lock-timeout` and adds
`error.details.retryable`, `next_action`, and bounded `recovery` metadata.
A failed create reports `retry-create` and `not_created` only after verified
runtime cleanup. Incomplete cleanup forbids blind create retry.

A resume provisioning timeout reports `retry-resume` and `prior_restored` only
when the new broker was proven not admitted, runtime cleanup was confirmed, and
the prior record, activity and startup artifacts were fully restored. That
never-admitted incarnation needs no revocation. Uncertain admission, cleanup or
restoration retains the rollback refusal; callers must inspect the same session.

Delete/archive retains the original addressable record on exhaustion and reports
`retry-delete`/`retry-archive` and `record_retained`; an exact retry completes the
persisted termination transition. It never retries the provider launch or
discards authority fences.
Writes use atomic replace and fsync ordering suitable for crash recovery.

The private store may contain bodies and capability digests. Public projections
are separately constructed and never serialize those fields.

## Coordination modes and automatic presence

Every managed session has one additive `coordination_mode`:

| Mode | Behavior |
| --- | --- |
| `advisory` | Default. Automatic presence and optional context produce privacy-safe warnings; overlap or coordination failure never denies work. |
| `enforce` | Opt-in. Raw claim coverage, atomic operation admission, and exclusive checkout writer semantics may deny mutation. |
| `off` | No agent-session collision warning or admission. Other safety, consent, delivery, intent, secret, and validation hooks are unaffected. |

Older `agent-session.session.v1` records without the additive field deserialize
as `advisory`. Managed tmux runtimes receive
`AGENT_SESSION_COORDINATION_MODE` alongside their session ID, state directory,
runtime ID, and capability path. Each broker projection persists the same mode;
older broker projections without the field default to `advisory`.

A ready, heartbeat-fresh broker plus its matching session record is an active
presence record. Presence begins during held launch, follows the exact runtime
incarnation, survives launcher exit, rotates on resume, and becomes inactive on
broker stop, target exit, or delete. No claim is required. A peer in `off` mode
does not participate. A process launched outside `agent-session` has no managed
identity and is outside this coordination universe.

Presence derives only:

- a private-keyed fingerprint of the canonical checkout root;
- the canonical `owner/repository` origin when available;
- the public managed session selector and mode; and
- optional explicitly declared provider and path context.

Raw checkout paths, capabilities, host/user identity, prompts, transcripts,
logs, terminal bytes, and mailbox bodies are never projected.

## Contract map

| Need | Canonical contract |
| --- | --- |
| Coordination schemas v1 | [Coordination schemas v1](coordination-schemas-v1.md) |
| Coordination mailbox v1 | [Coordination mailbox v1](coordination-mailbox-v1.md) |
| Coordination runtime and recovery v1 | [Coordination runtime and recovery v1](coordination-runtime-v1.md) |
| Coordination CLI and HTTP v1 | [Coordination CLI and HTTP v1](coordination-interface-v1.md) |
| Coordination federation and service messages v1 | [Coordination federation and service messages v1](coordination-federation-v1.md) |

## Versioned schemas

See [Versioned schemas](coordination-schemas-v1.md#versioned-schemas).

### Work context

See [Work context](coordination-schemas-v1.md#work-context).

### Conflict result

See [Conflict result](coordination-schemas-v1.md#conflict-result).

### Operation lease

See [Operation lease](coordination-schemas-v1.md#operation-lease).

### Operator provider-turn reconciliation

See [Operator provider-turn reconciliation](coordination-schemas-v1.md#operator-provider-turn-reconciliation).

### Message

See [Message](coordination-schemas-v1.md#message).

### Recipient notification policy

See [Recipient notification policy](coordination-schemas-v1.md#recipient-notification-policy).

### Broker status

See [Broker status](coordination-schemas-v1.md#broker-status).

### Managed first-heartbeat readiness

See [Managed first-heartbeat readiness](coordination-schemas-v1.md#managed-first-heartbeat-readiness).

## Scope grammar and canonicalization

Repositories are canonical lowercase `owner/name` values. Provider references
are `(kind, repository, numeric id)`. New plan references are rejected;
historical values remain readable in v1 projections and are ignored for
conflict matching.

V1 scope kinds are closed:

| Kind | Value | Overlap rule |
| --- | --- | --- |
| `repository` | fixed value `.` | Conflicts with every scope in the same repository. |
| `path-exact` | normalized repo-relative file/path | Conflicts with the same exact path and a covering prefix. |
| `path-prefix` | normalized repo-relative path without a trailing `/` | Conflicts with equal, ancestor, or descendant prefixes and contained exact paths at `/` boundaries. |

Unknown kinds are rejected. Empty, absolute, host-qualified, home-relative,
symlink-escaped, and dot-segment path values are rejected. Canonicalization is
byte-stable across CLI and HTTP.

Worktree values are non-reversible HMAC-SHA256 fingerprints using a private
registry key and a public key epoch. Raw checkout paths never enter the
registry projection. An unknown epoch is incomparable rather than clear.

### Conflict truth table

| Candidate versus peer | Result |
| --- | --- |
| Same active worktree fingerprint | `conflict` |
| Same provider ref | `conflict` |
| Same repository with overlapping closed scopes | `conflict` |
| Same repository with omitted, broad, or incomparable scopes | `potential_conflict` |
| Relevant live peer without valid/supported context | `unknown` |
| Complete relevant universe, all comparable and disjoint | `clear` |
| Incomplete universe with permissive projection requested and no known overlap | `no_known_conflict` |

The authenticated subject is excluded by exact session ID plus incarnation.
An explicit candidate is not removed merely because its fields resemble the
subject. Conflicting selectors or a missing subject for a self check fail.

## Relevant-peer universe

The authoritative claim transaction reads every non-expired managed session in
the selected registry snapshot. A peer is relevant when it is live or the
registry cannot safely establish terminality. Replaced, released, or expired
incarnations are retained for bounded audit but are not active conflicts.
Corrupt, oversize, future-schema, or partially upgraded peer records make the
view incomplete; they do not disappear from classification.

Standalone `check` and `advise` are advisory. Automatic presence includes every
ready managed peer even when no claim exists. Only raw `claim` combines
claim-to-claim evaluation and acquisition
under the same registry lock. Two concurrent definite contenders cannot both
receive an admitted claim.

## Authentication and authorization matrix

| Operation | Required authority |
| --- | --- |
| status/advise | current managed session capability when managed; an invocation without managed identity returns an explicit non-participating result |
| high-level set/clear/acknowledge | current managed session capability and incarnation, inferred from the environment |
| work-context show/session check/candidate check | public registry read; HTTP additionally requires the server operator token |
| self check/claim/renew/release | matching session capability and incarnation |
| operation admit/complete/reconcile | matching session capability, active claim, and execution token/proof |
| operator provider-turn reconcile over HTTP | server operator token only, exact current runtime/activity/turn selectors, confirmed inactive attestation, and quiescent authoritative broker |
| message send | matching sender capability |
| inbox/show/ack/reply/wait | matching recipient capability |
| broker status | public registry read; HTTP additionally requires the server operator token |
| broker adopt/reconcile | local lifecycle lock plus proof selectors matching an unchanged, live, exact persisted runtime whose broker is demonstrably lost |
| operator operation reconcile over HTTP | server operator token, exact current session incarnation/generation, exact nonterminal lease revision, confirmed inactive attestation, and no live exact descendant |
| HTTP registry-wide candidate check | server operator token; explicit subject/candidate rules still apply |

Capabilities rotate on resume/replacement and are revoked on delete/target exit.
Wrong principal, stale incarnation, wrong revision, wrong operation token, or
cross-principal idempotency reuse fails without revealing the expected value.
Recovery proof files contain only schema, incarnation, and generation selectors;
they never embed the server/operator token.

## Claim state machine

States are `active`, `stale`, `released`, and `expired`.

- `claim` validates the candidate, authenticates the subject, expires stale
  records, evaluates the complete snapshot, and creates one active 30-minute
  claim only when no `conflict` exists.
- `potential_conflict`, `unknown`, and `no_known_conflict` are returned as
  advisories in v1 and do not independently hard-block acquisition.
- `renew` requires claim ID, current revision, same incarnation, and a live
  broker. Heartbeat occurs before half the TTL.
- `release` is idempotent for the same principal/request and never affects a
  different incarnation. Release or replacement is rejected while a bound
  operation remains `active`, `completing`, or `reconcile_pending`.
- Broker loss marks the owner unavailable for new operations and eventually
  stales the claim. Pane liveness alone cannot renew a claim.

High-level `set` owns the ordinary mechanics: it infers the current session and checkout, canonicalizes
optional issue/PR/path/plan fields, reuses an unchanged active context idempotently, and replaces a
changed context without requiring a caller-supplied file, revision, claim ID, or idempotency key. In
advisory/off mode a definite overlap is returned but does not reject the declaration. In enforce mode it
retains raw claim conflict rejection. High-level `clear` is idempotent and still refuses to orphan a
nonterminal enforce operation.

`set --if-absent` is the atomic integration form: while holding the registry lock it returns an existing
active declaration unchanged, including its tier, references, scopes, summary, claim identity, and
revision; only a session with no active declaration creates the supplied context. It never replaces or
downgrades a concurrently established declaration.

Acknowledgement is keyed to the exact session incarnation and the most recent
canonical advisory observation, and expires after a caller-selected duration
of at most eight hours. It suppresses repeated hook rendering only when peer
incarnations, reasons, repositories, and availability are unchanged. Target
churn covered by the same known overlap remains suppressed to avoid per-file
warning spam; a target that changes the reason or repository warns again.
`advise` continues to return the actual reasons and severity.

## Operation lease state machine

States are `active`, `completing`, `reconcile_pending`, `completed`, `failed`, and `abandoned`;
`expired` is accepted only as a retained backward-compatible terminal state.

- `admit` re-evaluates peers atomically and proves every canonical filesystem
  scope and provider reference is a subset of the authenticated active claim
  before creating a 30-minute lease. Filesystem targets bind each repository to
  a canonical checkout whose `origin` matches the declared repository.
- Opaque repository effects, including shell operations, require an explicit
  repository scope covering the canonical checkout. Retired private grants do
  not widen an active claim.
- A 30-minute claim does not release a known long operation. Reaching the
  operation safety TTL moves `active` to fail-closed `completing`; it never
  asserts terminality or removes the bound claim's exclusion.
- A matching working activity identity or exact live descendant renews the
  operation. Pane/process-group liveness by itself never renews it.
- `complete` is idempotent from `active`, `completing`, or
  `reconcile_pending` and first persists the terminal tool result in a bounded
  broker-owned queue without raw stdout/stderr. The authenticated heartbeat
  sidecar drains that queue after caller loss; an event accepted before the
  safety TTL remains valid across the exact `active` to `completing` revision
  transition and across the exact original revision to `reconcile_pending`
  transition. Reconcile drains already-persisted completion events before it
  may advance an operation lease.
- `reconcile` repairs a missed completion only when the token digest matches and
  controller-owned state proves the unchanged exact persisted runtime stopped,
  or when two superseding-activity/no-descendant observations at least five
  seconds apart move `reconcile_pending` to terminal. A newer controller turn
  identity is superseding even while that new turn is working; progress within
  the same turn is not. Unknown activity and caller-supplied idle or descendant
  booleans are not accepted as proof.
- Uncertain heartbeat or proof blocks later owner operations and competing
  admission until validated recovery; it does not silently expire an active
  mutation.
- An expired lease is reclaimable. Registry maintenance renews a lease while its own turn is working or
  its exact descendant is live and the broker heartbeat is fresh, so a lease that reaches the safety TTL
  produced no renewal evidence for the whole TTL. Full registry maintenance, the counterpart of that
  renewal, first drains any persisted completion event of an expired nonterminal lease, whose outcome
  wins. It changes a remaining one to `abandoned` with outcome `ttl-expired-inactive` only when the lease
  belongs to its session's current incarnation, the unchanged exact persisted runtime still runs, and
  controller-owned evidence shows the lease's own turn superseded with no live descendant.

  Until then, admission fails with `coordination-unavailable` and notifications stay fenced. Reclaim
  needs no execution token and never changes the bound claim, so an idle worker whose lost lease blocked
  its own wake becomes deliverable again.
- Recovery evidence is turn-scoped. Both `reconcile` and the expired-lease
  reclaim treat a newer controller turn with no live exact descendant as proof
  that the lease's call is inactive; neither receives agent-scoped evidence.
  A mutation still running from a background subagent after its admitting turn
  ended, which also supplies no descendant identity, is therefore unsupported
  under `enforce` coordination: it can be finalized while it runs. Run
  mutations in the foreground turn that admitted them.
- Exact non-executing recovery uses authenticated `broker prepare-admission-proof`
  and `broker proof`, as specified in [Broker evidence v1](broker-evidence-v1.md).
  Preparation binds the existing admission request digest without issuing or
  replaying admission. Only an exact retained active operation with no outcome
  can restore its issued lease; uncertainty grants no execution authority.
- Exact `admit` replay remains supported for older consumers and producers.
  A guard replays the unchanged idempotency key, token and targets, then uses
  `complete` or `reconcile`. Proof-capable consumers prefer evidence to replay;
  neither aggregate counts nor missing retention prove terminality.

## Idempotency

Every mutation requires an idempotency key of 8 through 128 printable ASCII
bytes. Receipts bind principal, incarnation, operation, canonical request
digest, and outcome for 24 hours.

- Same key and same digest returns the original outcome.
- Same key with a different request under the same principal, incarnation, and
  operation returns `idempotency-key-reused` with no request content. The same
  raw key in another principal/incarnation/operation namespace is independent.
- Receipt cleanup is bounded and never removes a live claim, operation, or
  unread message needed to explain the retained outcome.

Receipt retention is bounded on two axes, and both are normative:

| Limit | Value | Scope |
| --- | --- | --- |
| Receipt count | 4,096 | per principal |
| Receipt count | 32,768 | whole registry |
| Aggregate serialized receipt bytes | 4 MiB | per principal |

The count limits bound how many outcomes a principal may retain; they do not
bound their size, because an outcome is an arbitrary JSON value. The byte
budget bounds the sum of one principal's serialized receipts, so a principal
holding far fewer receipts than the count quota cannot retain an unbounded
amount of memory or disk. It is recomputed from the registry rather than
cached, so it survives a broker restart unchanged.

The byte budget is per principal only; there is no registry-wide byte limit on
receipts. Enough distinct principals at full budget reach the whole-registry
cap, which surfaces as a failed registry write rather than a per-request
`quota-exceeded`. The registry cap, not the receipt budget, is the outer bound
on total retention.

At either per-principal axis, or at the whole-registry count, admission first
plans to evict that principal's oldest finished receipts (earliest expiry
first) until the new receipt fits, and applies the plan only when it is enough
to admit the receipt. A receipt
is finished when it has expired, or when its outcome names a `message_id` whose
message is `acknowledged`, `expired`, `deleted`, or no longer retained. Receipts
for `unread` or `read` messages, and receipts whose outcome names no message
(claims, operations), are never evicted. Evictions are counted cumulatively per
principal with the latest eviction time; a record idle for one receipt TTL is
removed lazily by a later eviction. A replay of an evicted key reads as a fresh request, so only finished
work is ever exposed to that, and only once the principal is full.

If the principal's finished receipts cannot make room, the store returns
`quota-exceeded` before the transition and evicts nothing. Replacing an existing receipt charges
only the difference, so a replayed request is not billed twice.

## Mailbox limits and state machine

See [Mailbox limits and state machine](coordination-mailbox-v1.md#mailbox-limits-and-state-machine).

### Same-session resume continuity

See [Same-session resume continuity](coordination-mailbox-v1.md#same-session-resume-continuity).

## Notification ownership

See [Notification ownership](coordination-mailbox-v1.md#notification-ownership).

## Managed launch and broker boundary

See [Managed launch and broker boundary](coordination-runtime-v1.md#managed-launch-and-broker-boundary).

### Operator recovery after an external runtime stop

See [Operator recovery after an external runtime stop](coordination-runtime-v1.md#operator-recovery-after-an-external-runtime-stop).

## CLI contract

See [CLI contract](coordination-interface-v1.md#cli-contract).

### Managed-session readiness

See [Managed-session readiness](coordination-interface-v1.md#managed-session-readiness).

## HTTP coverage

See [HTTP coverage](coordination-interface-v1.md#http-coverage).

## Public list and glance additions

See [Public list and glance additions](coordination-interface-v1.md#public-list-and-glance-additions).

## Stable failure codes

See [Stable failure codes](coordination-interface-v1.md#stable-failure-codes).

## Validation matrix

See [Validation matrix](coordination-interface-v1.md#validation-matrix).

## Cross-host mailbox federation v1

See [Cross-host mailbox federation v1](coordination-federation-v1.md#cross-host-mailbox-federation-v1).

## Owned child sessions v1

`agent-session start --via-console [--machine MACHINE]` starts a session through Agent Console instead of
launching tmux locally. Agent Console owns the new session for the same principal that owns the calling
managed session, so the child appears in that owner's console list and board and can be attached. The
command runs inside a managed session, with a scoped worker delegation, or as an admitted service
([Service Console starts v1](#service-console-starts-v1)): without any of them it fails with
`console-start-unmanaged`, and a missing or invalid capability for that session fails with
`coordination-unauthorized`.

It sends `agent`, the absolute `cwd` (default: the current directory), and the optional `title`, prompt
(`--prompt`, `--prompt-file`, or `--prompt-stdin`), `--agent-arg` values (for example a model),
`--agent-profile` as `agent_profile`, and `--account` as `codex_account` for `--agent codex` or
`claude_account` for `--agent claude`; `--account` with another agent fails with
`console-start-account-unsupported`. `--id`, `--tmux-bin`, `--agent-bin`, `--paste-delay-ms`, and
`--coordination-mode` conflict with `--via-console`, because Agent Console assigns the session id and the
target daemon owns the launch.

`--no-parent`, `--program`, `--issue`, and `--no-inherit-work` reach the daemon as `no_parent` (boolean)
and `work` (`{"program", "issues", "inherit"}`, references as JSON objects); see [Session lineage and
work v1](session-lineage-work-v1.md#console-starts).

As with federated messaging, the CLI reads the private
`coordination/daemon-endpoint.json` and calls its own daemon at
`POST /sessions/{id}/console-start/v1` with the current session capability. It
never reads relay secrets. The daemon authenticates the exact current
incarnation, then calls `POST {AGENT_SESSION_RELAY_URL}/api/coordination/sessions/v1`
with the relay token as bearer and this body:

```json
{
  "source_session_id": "caller",
  "source_incarnation": "caller launch UUID",
  "machine": "optional target machine",
  "session": {"agent": "claude", "cwd": "/abs/path", "title": "...", "prompt": "...",
              "lineage": {"…": "…"}, "work": {"…": "…"}}
}
```

The daemon route accepts `operation_id` (a canonical UUIDv7, optional),
`machine` (a nonempty string, optional),
`no_parent` (a boolean, optional), `work` (optional), and a `session` object;
anything else fails with `console-start-invalid` (HTTP 400) before any network
call. The daemon computes the child's `lineage` and resolved `work` from the
caller's record and sets them in `session`, replacing any the caller supplied
([Session lineage and work v1](session-lineage-work-v1.md#console-starts)). Neither the route nor the aggregator request has a
field that names an owner: the aggregator takes the owner from the caller's
exact Console grant. With federation unconfigured the route fails with
`console-start-disabled` (HTTP 409).

On success the daemon answers HTTP 201 with `{"schema_version": "agent-session.console-start.v1",
"machine", "session"}`, where `machine` is the requested machine or the daemon's own and `session` is the
aggregator's public projection of the created session, including its `id`. The CLI wraps it as
`cli.agent-session.console-start.v1`. An aggregator failure keeps its code and a bounded single-line
message when both have a safe shape: `ownership-unknown`, `machine-forbidden`, and
`session-incarnation-conflict` use the data exit class, and `invalid-request` the usage class.

An aggregator 401, a network failure, an unreadable body, or an unsafe code fails with
`console-start-unavailable` (HTTP 502). The request timeout is 120 seconds, because a create that pastes
a prompt or selects an account can take the target daemon over a minute. No lock is held across the
network call.

`console-start-unavailable` includes bounded `failure_stage` and
`failure_cause` values plus typed `retryable`, `next_action`, and `recovery`
details. A retry is safe only when `recovery.safe_to_retry` is true. Definite
pre-create failures may permit retry; an aggregator authentication rejection is
safe to repeat after configuration is repaired but is not itself retryable.
Transport failures, unreadable responses, and unconfirmed create results are
not safe to retry: inspect Console for the child before submitting again.
A target `start-failed` error preserves its allowlisted startup reason, exit
code, bounded redacted diagnostic, retained session ID, and cleanup/retry policy.
The `--via-console` client preserves only recognized failure fields and treats
missing or malformed details as unsafe.

### Operation diagnostics and reconciliation

See [Operation diagnostics and reconciliation](coordination-runtime-v1.md#operation-diagnostics-and-reconciliation).

## Service Console starts v1

An explicitly admitted deterministic service can start a Console-owned root session for an owner
without a model turn and without another session's runtime credential. This is a separate authority
from managed-session starts, scoped worker delegations and mailbox service submission; none of them
grants it, and it grants none of them.

```bash
agent-session start --via-console --service ID --service-generation GENERATION \
  --credential-file FILE --owner OWNER --machine MACHINE --expires-at TIMESTAMP \
  --operation-id UUIDV7 --agent claude --cwd /abs/path [--title ...] [--prompt-file FILE] --format json
agent-session start --via-console --service ID --service-generation GENERATION \
  --credential-file FILE --owner OWNER --operation-id UUIDV7 --reconcile --format json
```

`--service` requires `--via-console`, `--service-generation`, `--credential-file` and `--owner`, and
conflicts with `--no-parent`, `--role`, `--program`, `--issue`, `--no-inherit-work` and `--resume`.
`--machine` and `--expires-at` are required to create and are refused with `--reconcile`. The other
session options are those of [Owned child sessions v1](#owned-child-sessions-v1). The credential is
read from a private file, never from argv or the environment.

### Admission

`serve --console-start-services-file FILE` (fallback `AGENT_SESSION_CONSOLE_START_SERVICES_FILE`)
references a private regular file (mode `0600`, one link, at most 32 KiB), re-read on every request.
Omission disables service starts. It is separate from the mailbox service document:

```json
{
  "schema_version": "agent-session.console-start-services.v1",
  "services": [
    {
      "service_id": "integration",
      "service_generation": "generation-1",
      "credential_file": "/absolute/private/integration.credential",
      "owners": ["owner-a"],
      "machines": ["host-a"],
      "max_lifetime_seconds": 3600
    }
  ]
}
```

IDs, generations and owners are 1-128 ASCII letters, digits, `-`, `_` or `.`; machine names are 1-128
printable ASCII characters without spaces. At most 32
services are admitted, with unique IDs; each lists 1-64 owners and 1-64 machines, and
`max_lifetime_seconds` is 1-86400. The credential file holds a 32-256 byte printable ASCII token and
must differ from the operator bearer, the federation relay and ingress tokens, and every mailbox
service credential.

### Route and authorization

`POST /coordination/services/console-start/v1` accepts only a direct loopback peer, with the service
credential as the bearer, at most 8 concurrent starts, a 5-second body read and a 2 MiB body. The
body rejects unknown members:

```json
{
  "service_id": "integration", "service_generation": "generation-1", "owner": "owner-a",
  "operation_id": "UUIDv7", "machine": "host-a", "expires_at": "2030-01-01T00:10:00Z",
  "session": {"agent": "claude", "cwd": "/abs/path", "title": "...", "prompt": "..."}
}
```

`session` may set only `agent`, `cwd`, `workspace`, `launch_env`, `title_mode`, `title`, `prompt`,
`agent_args`, `agent_profile`, `codex_account` and `claude_account`. The daemon sets the lineage: a
root with `parent: null` and `starter {"kind": "console", "via": "console"}`, no forge context and no
origin parent. A reconcile body is exactly `service_id`, `service_generation`, `owner`,
`operation_id` and `reconcile: true`.

A request is authorized only when, in this order and before any receipt or network call, the body is
a closed request; the service ID, generation and credential match one admitted entry; `owner` is in
its `owners`; `machine` is in its `machines`; and `expires_at` is in the future and within
`max_lifetime_seconds`. With federation unconfigured the route fails with `console-start-disabled`.
Owner resolution stays with the aggregator.

| Code | Class | HTTP | Cause |
| --- | --- | --- | --- |
| `console-start-service-unauthorized` | data/65 | 401 | disabled, missing or invalid admission, unknown service or generation, wrong or shared credential, non-loopback peer, or a session capability |
| `console-start-service-owner-denied` | data/65 | 422 | `owner` not admitted for the service |
| `console-start-service-target-denied` | data/65 | 422 | `machine` not admitted for the service |
| `console-start-service-expired` | data/65 | 422 | `expires_at` past or beyond the admitted lifetime |
| `console-start-service-invalid` | usage/64 | 400 | unknown or missing member, forbidden `session` member, malformed selector, UUIDv7 or timestamp |

Refusals are content-free: they never echo paths, credentials, prompts or upstream text, and the
unauthorized code does not say which check failed. The existing `console-start-operation-conflict`,
`-not-found`, `-expired`, `console-start-disabled` and `console-start-unavailable` codes apply unchanged.

### Aggregator contract

The daemon creates with `POST {AGENT_SESSION_RELAY_URL}/api/coordination/sessions/v1`, the relay
token as bearer and `x-agent-session-operation-id`, and this body. It has no `source_session_id` or
`source_incarnation`:

```json
{
  "source_service_id": "integration", "source_service_generation": "generation-1",
  "owner": "owner-a", "machine": "host-a",
  "session": {"agent": "claude", "cwd": "/abs/path", "lineage": {"…": "…"}}
}
```

An uncertain operation is reconciled with one read,
`GET {AGENT_SESSION_RELAY_URL}/api/coordination/board/v1?source_service_id=ID&source_service_generation=GENERATION&owner=OWNER&machine=MACHINE`,
and a candidate is reported only when its record has `machine`, a `lineage` object whose `parent`
is null (a record without `lineage` is not a root) and `console_owner` equal to the request. The relay
must bind the selectors and `owner` to the authenticated machine's explicit service admission before
it creates or returns anything.

The aggregator's answer is mapped as for session-scoped starts, and there is no fallback to the
session-scoped call:

- An unadmitted service, owner or target is answered with `console-start-service-unauthorized`,
  `console-start-service-owner-denied` or `console-start-service-target-denied`.
- Any typed usage- or data-class aggregator code, including those and a rejection of the request
  shape such as `invalid-request`, is forwarded with its own code and records the operation
  `not_created` with `safe_to_retry: true`.
- Only a missing or unreadable answer is `console-start-unavailable`; it leaves the operation
  `ambiguous`, to be reconciled.

### Receipt and idempotency

The operation receipt store of [Owned child sessions v1](#owned-child-sessions-v1) records the
service principal and owner. Its key is the service machine, service ID and operation ID, so a
reconcile survives a generation rotation; the submitting generation is recorded. A receipt belongs to
the owner it was started for: reconciling it under another owner is `console-start-operation-not-found`.

- The first request stores an `ambiguous` receipt under the store lock, then makes at most one
  aggregator create call.
- A retry must resend the identical request, including `expires_at`: the fingerprint covers `owner`,
  `machine`, `expires_at` and `session`, and not the operation ID or the service generation. A
  different request under the same operation ID fails `console-start-operation-conflict`.
- Repeating the request returns the stored outcome and never posts again, including after `expires_at`
  has passed; expiry is checked only for an operation with no receipt. An `ambiguous` repeat returns
  the reconciliation, never a second create.
- `--reconcile` is read-only and never adopts a candidate.

Every success, replay and reconciliation carries `receipt`:

```json
{
  "schema_version": "agent-session.console-start-service-receipt.v1",
  "service_id": "integration", "service_generation": "generation-1", "owner": "owner-a",
  "machine": "host-a", "operation_id": "UUIDv7",
  "state": "created", "safe_to_retry": false,
  "session": {"id": "...", "agent": "claude", "session_incarnation": "..."}
}
```

`state` is `created`, `ambiguous`, `not_created` or `start_failed`; `session` is `null` until known.
A consumer keeps its previous session unless `state` is `created` with a session ID, and rebinds its
route only then. Operation diagnostics and the 7-day receipt retention are those of
[Operation diagnostics and reconciliation](coordination-runtime-v1.md#operation-diagnostics-and-reconciliation).

## Service-origin submission

See [Service-origin submission](coordination-federation-v1.md#service-origin-submission).

## Message categories and recipient forwarding

See [Message categories and recipient forwarding](coordination-federation-v1.md#message-categories-and-recipient-forwarding).

### Mixed-version rollout

See [Mixed-version rollout](coordination-federation-v1.md#mixed-version-rollout).

## Scoped worker delegations

[Scoped worker delegation v1](scoped-worker-delegation-v1.md) defines a separate
private credential for supervised workers. Only console-start and message-send
admission accept it, and enforce its operation/recipient scope, expiry, and the
current delegating incarnation. Ordinary runtime capabilities retain their
existing authority.
