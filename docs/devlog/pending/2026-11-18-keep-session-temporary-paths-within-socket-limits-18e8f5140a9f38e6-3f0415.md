## 2026-11-18 - Keep session temporary paths within socket limits
<!-- devlog-id: 2026-11-18-keep-session-temporary-paths-within-socket-limits-18e8f5140a9f38e6-3f0415 -->

### Result

- Managed sessions receive a short private path to their existing temporary
  directory.

### Why / context

- Socket clients need a bounded path while the existing owner retains isolation
  and reclaim responsibilities.

### Evidence

- A synthetic Chrome socket regression failed before the change; session and
  launch fixture tests passed after it.

### Links

- <https://github.com/sympoies/agent-runtime-kit/issues/314>
