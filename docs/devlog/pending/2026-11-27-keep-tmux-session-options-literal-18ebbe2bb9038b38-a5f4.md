## 2026-11-27 - Keep tmux session options literal
<!-- devlog-id: 2026-11-27-keep-tmux-session-options-literal-18ebbe2bb9038b38-a5f4 -->

### Result

- Escape tmux format markers in provider session names and working directories.

### Why / context

- tmux expands session and directory option values before starting a session.

### Evidence

- A fake tmux regression covers fresh, resumed, and one-shot launches; a
  private tmux probe confirmed literal values.

### Links

- <https://github.com/sympoies/nils-cli/issues/4132>
