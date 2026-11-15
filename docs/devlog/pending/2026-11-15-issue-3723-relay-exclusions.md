## 2026-11-15 - Explain relay ownership exclusions before mailbox submission
<!-- devlog-id: 2026-11-15-issue-3723-relay-exclusions -->

### Result

- Peer JSON preserves relay exclusions separately from authorized peers; text
  labels exclusions and sends return remote-recipient-excluded with the relay
  reason.

### Why / context

- A stale ownership binding intentionally denies discovery. Distinguishing that
  denial from an absent address lets the owner use the documented
  incarnation-fenced check/apply recovery without widening authorization.
  Unknown reasons remain opaque and do not inherit stale-binding recovery.

### Evidence

- Synthetic relay regressions first failed on excluded text labels and the
  generic send error (2 failed, 2 passed); the focused suite then passed all 4
  tests. Coverage includes missing fields, unknown reasons, exact addressing,
  authorized-peer priority and no outbox persistence on exclusion.

### Links

- <https://github.com/sympoies/nils-cli/issues/3723>
