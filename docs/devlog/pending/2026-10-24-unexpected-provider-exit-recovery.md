## 2026-10-24 - Expose unexpected provider exits with explicit recovery
<!-- devlog-id: 2026-10-24-unexpected-provider-exit-recovery -->

### Result

- Session and board views retain bounded provider exit reasons and an explicit
  Resume action without automatic restarts.

### Why / context

- A provider interrupt or quit previously looked like an unexplained stopped
  session.

### Evidence

- Fake-provider SIGINT regression and lifecycle tests; focused crate
  validation.

### Links

- Refs #2602
