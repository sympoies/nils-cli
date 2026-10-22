## 2026-10-23 - Diagnose removed pinned session helpers
<!-- devlog-id: 2026-10-23-diagnose-removed-pinned-session-helpers-18e0ebdc055f1b8a-2f41c6 -->

### Result

- Activity doctor lists running sessions whose tmux-pinned agent-session binary
  path is missing, in text and JSON.

### Why / context

- A running session can retain a helper path removed after an installation
  upgrade.

### Evidence

- A removed-helper fixture captures the missing diagnostic before
  implementation and checks running, stopped, and existing-helper cases.

### Links

- Refs #2672
