## 2026-10-26 - Keep incremental session previews current
<!-- devlog-id: 2026-10-26-issue-2943-session-message-previews -->

### Result

- Session lists observe accepted queued input and advertise an optional latest
  user or assistant message from the same tracker.

### Why / context

- Claude queue records were acknowledged without updating previews, while
  metadata-only warm checks could continually return pending between background
  catch-ups. Bounded warm parsing fixes that scheduling gap while preserving
  async cold recovery and truthful backlog states.

### Evidence

- Synthetic regression RED reproduced both gaps; focused provider tracker tests
  (55) and last-prompt HTTP/discovery tests (17) passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2943>
