## 2026-10-22 - Recognize rewritten worktree deliveries and path holders
<!-- devlog-id: 2026-10-22-recognize-rewritten-worktree-deliveries-and-path-holders-18e0d12e1a804873-296068 -->

### Result

- Add opt-in merged pull request delivery evidence and filter lsof holders by
  reported path.

### Why / context

- Squash merges rewrite commit identity and shared hard links can create
  unrelated inode matches.

### Evidence

- Focused squash delivery and hard-link holder regressions pass.

### Links

- Refs #2777
