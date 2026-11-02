## 2026-11-02 - Pin the screen reader with a recorded pane corpus
<!-- devlog-id: 2026-11-02-real-pane-corpus-frozen-classifier -->

### Result

- A recorded corpus of one sanitized real Claude Code 2.1.296 pane and four
  named edits of it now pins what the composer reader returns, including the
  typed reason for each refusal. It runs in the normal suite.
- The reader itself is unchanged. Escape bytes in every capture are intact, and
  a second test fails if a capture holds a home path, scratch path or an at
  sign.

### Why / context

- Refs 3186: the reader infers composer content from screen text, so it is
  frozen and an unknown form must stay a typed refusal. The corpus makes that
  checkable, and gives later real captures one place to land.

### Evidence

- The first run passed on arrival because the corpus records current behavior.
  A throwaway change to one expected reason made the test fail and name the
  entry, and the expectation was restored.

### Follow-ups

- The corpus holds one provider build and no Codex pane of its own. Captures
  from the tested range, and any raw capture for 3201, are added only after
  sanitizing. This does not prove the reader on builds it does not hold.
