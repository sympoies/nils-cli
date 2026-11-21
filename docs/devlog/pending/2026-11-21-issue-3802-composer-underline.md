## 2026-11-21 - Read tmux extended underline in guarded composer captures
<!-- devlog-id: 2026-11-21-issue-3802-composer-underline -->

### Result

- Guarded composer reads handle tmux extended underline SGR without changing
  dim state.

### Why / context

- A curly-underlined transcript row above the composer could make the entire
  pane unreadable and refuse a guarded send at admission.

### Evidence

- Retained pre-edit RED reproduced pane-unparsable. Synthetic frame and private
  fake tmux fixtures cover exact text, draft refusal, and post-paste receipt
  stages.

### Links

- <https://github.com/sympoies/nils-cli/issues/3802>
