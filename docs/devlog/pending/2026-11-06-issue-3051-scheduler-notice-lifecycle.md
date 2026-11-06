## 2026-11-06 - Deliver scheduler continuations as attested notices
<!-- devlog-id: 2026-11-06-issue-3051-scheduler-notice-lifecycle -->

### Result

- Three recovery producers use canonical notices and receipt-backed machine
  history; keyed native busy replies carry their client id, and generated
  bodies prevent line-trailing ASCII spaces.
- Scheduler admission refuses known stale drafts without operator repair.

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

### Links

- <https://github.com/sympoies/nils-cli/issues/3051>
- <https://github.com/sympoies/nils-cli/issues/3393>

### Follow-ups

- The coordinator owns installed native acceptance. Existing mailbox/operator
  Codex attestation is separate; no release, deployment or writer enablement is
  included.
