## 2026-11-06 - Explain review-loop ceiling recovery
<!-- devlog-id: 2026-11-06-review-loop-ceiling-recovery -->

### Result

- Inspect exposes a state-bound approval proposal at the no-progress ceiling,
  and the assigned writer can consume a subsequent authorized approval.

### Why / context

- The ceiling precedes a durable stop, so obtaining approval previously required
  another reviewed head; the proposal now preserves the existing state binding.

### Evidence

- An offline fixture exercises ceiling inspection without a provider write.

### Links

- <https://github.com/sympoies/nils-cli/issues/3229>
