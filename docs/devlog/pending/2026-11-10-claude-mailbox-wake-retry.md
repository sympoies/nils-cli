## 2026-11-10 - Retry unoffered Claude mailbox wakes
<!-- devlog-id: 2026-11-10-claude-mailbox-wake-retry -->

### Result

- Keep unoffered Claude mailbox wakes queued across composer clearing and retry
  after native receiver authorization.

### Why / context

- A receiver timeout previously parked a generation before notice text was
  released.

### Evidence

- Focused terminal-mailbox suite, clippy, formatting, and strict file-size audit passed.

### Links

- Refs #3484
