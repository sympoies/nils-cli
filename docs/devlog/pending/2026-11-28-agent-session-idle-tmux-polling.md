## 2026-11-28 - Cut idle agent-session tmux polling
<!-- devlog-id: 2026-11-28-agent-session-idle-tmux-polling -->

### Result

- Batch daemon inventory, back off quiet terminal observations, and resolve the
  short hostname in process without changing live input or retirement guards.

### Why / context

- Idle discovery and terminal sampling need not fork for absent consumers or
  unchanged pane output.

### Evidence

- The 20-session fake-tmux fixture reduced starts from 178 to 8 per minute and
  daemon CPU from 1.70 s to 0.93 s in the same 60-second idle window.

### Links

- <https://github.com/sympoies/nils-cli/issues/4212>
