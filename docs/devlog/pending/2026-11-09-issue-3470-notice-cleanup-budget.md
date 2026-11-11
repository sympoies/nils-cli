## 2026-11-09 - Bound wrapped notice cleanup by the terminal guard budget
<!-- devlog-id: 2026-11-09-issue-3470-notice-cleanup-budget -->

### Result

- Codex notice cleanup has time to complete its existing cursor proof and still
  refuses safely when observation expires.

### Why / context

- A wrapped notice requires many tmux round trips; the shorter cleanup budget
  could expire before deletion on a slow runner.

### Evidence

- A delayed capture fixture reproduced the idle-conflict refusal before the
  change; a slower fixture preserves the paste when the terminal guard budget
  expires.

### Links

- <https://github.com/sympoies/nils-cli/issues/3470>
