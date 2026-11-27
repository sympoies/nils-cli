## 2026-11-28 - Stabilize native prompt receipt reconciliation tests
<!-- devlog-id: 2026-11-28-issue-4177-native-reconciliation -->

### Result

- Use a scoped test transport budget and explicit provider disconnect for
  native reconciliation fixtures.

### Why / context

- Scheduling delays could exhaust the original test caps before the no-ack
  fixture recorded one mutation. Explicit replies keep the test focused on what
  receipt reconciliation accepts.

### Evidence

- Controlled pane and provider delays reproduce both baseline failures. The
  three selected reconciliation tests pass with the original receipt and
  mutation assertions.

### Links

- <https://github.com/sympoies/nils-cli/issues/4177>
