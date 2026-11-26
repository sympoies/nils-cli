## 2026-11-27 - Preserve literal tmux launch arguments
<!-- devlog-id: 2026-11-27-issue-4122-tmux-launch-arguments -->

### Result

- Encode session launch values for tmux at the shared launch boundary.

### Why / context

- A provider argument must reach the provider with its original bytes intact,
  including punctuation that tmux parses in command lines.

### Evidence

- A fake tmux process captures argument bytes and checks the explicit launch
  environment.

### Links

- <https://github.com/sympoies/nils-cli/issues/4122>
