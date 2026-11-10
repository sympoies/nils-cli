# Recover an unavailable review publisher

Use this route when an assigned reviewer's publishing route is unavailable.
The recorded coordinator owns the decision. A reviewer cannot release their
own assignment, and mailbox verdicts never count as provider review evidence.

Read the PR's current head and trusted state tip:

```bash
forge-cli pr review-handoff inspect "$PR" --expected-head "$HEAD_SHA"
```

As the recorded coordinator, release the assignment using those exact values:

```bash
forge-cli pr review-handoff recover "$PR" \
  --reason publisher-unavailable \
  --expected-head "$HEAD_SHA" \
  --expected-state "$STATE_TIP"
```

Recovery appends the reason to the trusted chain and advances the assignment
generation. A head or tip mismatch requires fresh inspection before retrying.
It creates no review or observation. The returned status remains
`returned-to-coordinator` and the handoff records `publisher-unavailable`.

Merge follows the repository's non-assigned review policy after this release.
If that policy permits delivery without enforced review convergence, use the
existing merge option:

```bash
forge-cli pr merge "$PR" --review-convergence=false
```

Required checks, existing review findings and hard stops, head/base and chain
CAS, holds, threads, tasks and provider policy still apply. Default or enforced
review convergence retains its ordinary evidence requirements. No self-review,
fabricated observation or direct provider merge is part of this recovery.

`reviewer-*` recovery reasons and surrender continue to revoke ownership with
`designated_reviewer_unavailable`. Designated checks and publication also remain
unavailable after publisher recovery. To resume designated review, the recorded
coordinator can `assign` using the current provider base/head and returned state
tip; the fresh generation requires its own reviewer observation and publication.

See the [handoff contract](../specs/forge-review-state-v1.md) for identity,
trusted-chain and recovery rules.
