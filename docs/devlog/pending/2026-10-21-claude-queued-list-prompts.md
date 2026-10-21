## 2026-10-21 - Read list-form Claude queued prompts in history
<!-- devlog-id: 2026-10-21-claude-queued-list-prompts -->

### Result

- agent-session history now reads a `queued_command` attachment whose prompt is
  a list of content blocks: text blocks are joined and image blocks count as
  the row's images, so a busy-time reply with an image appears in history;
  string prompts are unchanged.

### Why / context

- Claude Code records an image reply sent while a turn runs as a list-form
  queued prompt; the string-only reader dropped the row, so a delivered reply
  looked unconfirmed (#2624).

### Evidence

- Synthetic fixtures for text plus image, image only, string, and non-human
  list prompts in
  `provider_history/history_images/queued_list_prompt_tests.rs`; the list-form
  cases failed before the change.

### Links

- #2624

### Follow-ups

- The queued hand-off receipt reads the tmux pane, not the transcript, so it is
  unchanged here.
