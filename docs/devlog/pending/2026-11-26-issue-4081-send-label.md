## 2026-11-26 - Fence local agent-session sends by workflow label
<!-- devlog-id: 2026-11-26-issue-4081-send-label -->

### Result

- Add a workflow-owned label precondition to terminal and local mailbox sends,
  with a typed refusal for remote mailbox destinations.

### Why / context

- A routing snapshot can become stale before delivery. Comparing and committing
  under the destination record lock serializes delivery with label rotation.

### Evidence

- The baseline reproduced stale enqueue. Focused nextest cases verified stale
  and missing refusal, normal delivery, replay, and deterministic writer
  contention.

### Links

- <https://github.com/sympoies/nils-cli/issues/4081>
