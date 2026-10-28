## 2026-10-29 - Report persisted review handoffs when verification fails
<!-- devlog-id: 2026-10-29-issue-3020-handoff-persistence -->

### Result

- Acknowledged handoff writes report an explicit persisted-state error, exact
  written tip, and inspect recovery when post-write verification fails. Normal
  genesis assignment still succeeds.

### Why / context

- A provider-accepted assignment can outlive a failed read-back. A generic
  error without its digest invites unsafe retries; pre-write scope, ownership,
  tip and generation fences remain intact.

### Evidence

- Captured the read-back regression before production edits; the baseline
  normal genesis path already passed. Focused handoff/test-integrity validation
  passed 34 unit and 128 integration cases using synthetic providers. Full
  gates remain CI-owned.

### Links

- <https://github.com/sympoies/nils-cli/issues/3020>
