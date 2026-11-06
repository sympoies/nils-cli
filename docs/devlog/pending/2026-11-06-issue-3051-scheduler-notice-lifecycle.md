## 2026-11-06 - Deliver scheduler continuations as attested notices
<!-- devlog-id: 2026-11-06-issue-3051-scheduler-notice-lifecycle -->

### Result

- Three recovery producers use canonical notices and receipt-backed machine
  history; keyed native busy replies carry their client id, and generated
  bodies prevent line-trailing ASCII spaces.
- Scheduler admission refuses known stale drafts without operator repair,
  and the native receiver repeats that observation after queue handoff before
  its one write, retaining the original deadline.

### Why / context

- Native ingress requires releasing the session record lock. A separate
  process-held attempt fence preserves duplicate and crash semantics while
  exact runtime, account, predecessor and activity bindings are revalidated
  before input.

### Evidence

- Verified pre-production RED for producer routing, history role, generated
  formatting and busy ids. Focused receiver GREEN includes Claude lock handoff,
  Codex non-idle capacity and the duplicate-tick regression. Native acceptance
  remains unclaimed.
- Post-review queue-handoff RED emitted one native input after a draft appeared;
  the identical fixture is GREEN after the receiver repeats preserving-draft
  admission before its write.

### Links

- <https://github.com/sympoies/nils-cli/issues/3051>
- <https://github.com/sympoies/nils-cli/issues/3393>

### Follow-ups

- The coordinator owns installed native acceptance. Existing mailbox/operator
  Codex attestation is separate; no release, deployment or writer enablement is
  included.
