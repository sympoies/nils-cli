## 2026-10-26 - Conditional session deletion preserves unread mail
<!-- devlog-id: 2026-10-26-issue-2933-atomic-unread-delete -->

### Result

- Added opt-in --if-unread-empty to stopped/incarnation-fenced delete with
  typed unread and unknown-mailbox refusals.

### Why / context

- An empty list snapshot can become stale before deletion without changing the
  runtime incarnation. Message admission and conditional deletion now share the
  recipient lifecycle fence through removal.

### Evidence

- A real message delivered after an empty list snapshot reproduced successful
  deletion before the fix. Ten focused deletion tests and four CLI contract
  tests pass; completion behavior, shell syntax, and strict file-size checks
  pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2933>
