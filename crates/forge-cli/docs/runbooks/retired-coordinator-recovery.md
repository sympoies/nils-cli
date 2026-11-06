# Recovering a retired review coordinator

A supervisor may take ownership only after the session authority proves that
the recorded coordinator is stopped or deleted and its last terminal activity
is older than the repository policy threshold. A missing or unreachable session
is not retirement proof. Recovery preserves findings and revokes the old review
interval; it does not grant a review verdict.

## Configure a supervisor

Commit policy to the provider base branch:

```toml
[review_handoff]
supervisors = ["role:steward"]
stale_after = "2h"
```

The trusted operator launcher must start each supervisor with a forge initiator,
its role, and explicit `--attest-role`. The authenticated broker binding then
carries `attested_role`. The role selector survives replacement sessions, but
each session needs its own attestation. Existing sessions cannot gain one by
changing metadata, restarting a runtime, or asserting a role in the environment.
An ordinary `--role steward` or board role is insufficient; managed children
cannot inherit or self-assign operator provenance.

The authority boundary is agent-session's trusted launcher under the same OS
user. It does not isolate arbitrary code that can impersonate that launcher or
modify its private registry. A same-user process deliberately leaving its managed
environment or replacing the broker executable is outside this guarantee.
Keep launcher provisioning under operator control.

A policy may instead list exact `sha256:` session digests. Those entries need
no migration and require no role binding. Mixed policies prefer an exact digest
match. Neither global configuration nor changes on the PR head authorize a
supervisor. The actor must differ from the recorded coordinator and reviewer.

## Inspect and recover

Read the PR's current head/base and `pr review-handoff inspect` state. Fetch the
selected remote's base branch so its cached commit exactly matches the provider
base. Retain the current ledger tip, then run:

```bash
forge-cli pr review-handoff recover "$PR" \
  --reason coordinator-retired \
  --base-sha "$BASE_SHA" \
  --expected-head "$HEAD_SHA" \
  --expected-state "$TIP_DIGEST" \
  --dry-run
```

The fresh complete session board must prove all matching coordinator rows
terminal on one available source. Role policy additionally authenticates the
acting session's broker capability and current incarnation. A live coordinator,
unattested or wrong role, stale binding, or incomplete retirement proof refuses
recovery. Inspect the returned prerequisite before retrying.

After a successful preview, repeat with the same selectors and without
`--dry-run`. The CLI rechecks provider scope, ledger ownership, retirement proof,
and role attestation before append. The ledger retains the acting session digest,
retirement evidence, attested role, and digest of the full authenticated binding;
raw session IDs and machine names remain private. Assign a new review interval
normally after recovery and obtain a fresh reviewer-owned observation.

See the [handoff contract](../specs/forge-review-state-v1.md) for refusal codes,
source ambiguity, generation fencing, and concurrent writer handling.
