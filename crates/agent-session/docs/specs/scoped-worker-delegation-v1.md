# Scoped worker delegation v1

A managed caller can authorize a separately supervised worker without sharing
its runtime capability or checkpoint. This contract is provider independent and
supports Codex and Claude runtimes.

## Mint and use

Mint with the caller's ordinary runtime credential:

```bash
agent-session delegation mint --session "$AGENT_SESSION_ID" \
  --operation start-via-console --operation message-send \
  --recipient "$RECIPIENT_MACHINE/$RECIPIENT_SESSION_ID" \
  --expires-in 3600 --format json
```

`data.credential_file` references a newly created private mode `0600` credential.
The response also includes `id`, `session_id`, `session_incarnation`, and
`expires_at_epoch`. It never includes the credential contents. The worker
receives this file reference through private supervisor configuration; job
records and logs must never contain credential contents. It must use the same
local state directory and user as the caller.

Configure `AGENT_SESSION_CAPABILITY_FILE` to reference that delegation file in
the worker's environment, then use the ordinary commands:

```bash
agent-session start --via-console --agent codex --format json
agent-session message send --from "$CALLER_SESSION_ID" \
  --to "$RECIPIENT_SESSION_ID" --to-machine "$RECIPIENT_MACHINE" \
  --body-file "$RECEIPT_FILE" --idempotency-key "$JOB_RECEIPT_KEY" --format json
```

The start derives its caller from the delegation, without requiring
`AGENT_SESSION_ID` or a runtime checkpoint in the worker. Mailbox sends also
accept the delegation path through their existing `--capability-file` flag.
Console starts still require a configured local daemon and federation. Existing
lineage, ownership, destination discovery, idempotency and quota rules apply.

`--operation` is repeatable and accepts only `start-via-console` and
`message-send`. A message scope requires at least one repeatable `--recipient`
in exact `MACHINE/SESSION_ID` form. Names are literal, with no prefixes,
wildcards or groups; local destinations use the configured local machine
identity. Local sends must also match the issuer machine recorded at mint;
changing the worker process machine label cannot redirect a remote scope locally.
Recipients without `message-send` are refused. A start scope permits
console starts and operation reconciliation. Other commands, including mailbox
reads, acknowledgements, forwarding, and delegation mint/revoke, require the
ordinary caller credential. A worker cannot redelegate.

`--expires-in` is an integer lifetime in seconds, default `3600`, range
`1..=86400`. At most 128 unexpired, unrevoked delegations per caller and 4096
retained delegations globally are admitted. Minting removes registry records
expired for more than one day; callers may remove expired credential files
through their normal private credential-file lifecycle.

## Verification and revocation

Each admitted boundary checks the presented credential hash, session id, exact
runtime incarnation, expiry, operation and exact recipient. The caller's broker
must remain ready with a fresh heartbeat and its original runtime credential
available. Local sends recheck while holding the mailbox commit lock; remote
sends recheck after discovery and before outbox persistence. The daemon checks
console-start and remote-send scopes independently of CLI checks.

The caller can revoke one delegation explicitly:

```bash
agent-session delegation revoke --session "$AGENT_SESSION_ID" \
  --id "$DELEGATION_ID" --format json
```

Ending the caller, revoking or replacing its credential, or replacing the
runtime incarnation invalidates its delegations automatically. Revocation
prevents new submissions and replays; it does not cancel an already admitted
console start or remove an already queued mailbox message.

Typed refusals use the usual command JSON envelope and data-error exit status:

| Code | Meaning |
| --- | --- |
| `delegation-unauthorized` | Unknown or forged delegation |
| `delegation-scope-denied` | Operation, caller or recipient outside scope |
| `delegation-expired` | Expiry reached |
| `delegation-revoked` | Explicit revocation or caller unavailable/revoked |
| `delegation-incarnation-replaced` | Caller runtime incarnation changed |
| `delegation-invalid` | Invalid operation/recipient combination (usage error) |
| `delegation-quota-exceeded` | Delegation registry capacity reached |

Untrusted private-file references retain `coordination-unauthorized`. Ordinary
capability-only consumers refuse delegation credentials; they never promote a
worker to an unrestricted session principal.

## Private audit

The first delegation or delegation audit upgrades the private coordination
registry to `agent-session.coordination-registry.v4`. Later writes keep that
fence even after delegation records expire or are removed, including writes
that also carry notification suppression. Older writers refuse this version
before rewriting; upgrade every registry writer and projection reader for the
state directory before minting. Rollback requires a compatible binary; there
is no automatic downgrade. Existing registries without worker state retain
their current schema version.

The private coordination registry stores delegation metadata and credential
hashes, never credential contents. It retains the newest 4096 delegation-use
entries in `delegation_audit`: delegation id, operation, recipient hash (if any),
allow/refusal code and epoch timestamp. Recognized credential attempts are
recorded at console-start and message-send admission, including scope, expiry,
and revocation refusals. Audit persistence must succeed before operation
admission. An `allowed` entry records authorization, not successful completion;
ordinary operation receipts own completion. Successful CLI prechecks and commit-lock rechecks do not duplicate the daemon
admission entry; rejected CLI prechecks are audited immediately. Bodies, credentials,
checkpoint contents and supervisor/job configuration are excluded.
