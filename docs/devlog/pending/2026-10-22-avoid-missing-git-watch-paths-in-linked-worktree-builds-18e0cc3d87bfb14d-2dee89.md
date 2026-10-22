## 2026-10-22 - Avoid missing Git watch paths in linked worktree builds
<!-- devlog-id: 2026-10-22-avoid-missing-git-watch-paths-in-linked-worktree-builds-18e0cc3d87bfb14d-2dee89 -->

### Result

- Watch existing Git metadata paths so no-op linked-worktree builds preserve
  artifact identity while HEAD and packed tag changes invalidate version
  information.

### Why / context

- Missing per-worktree refs paths forced repeated Cargo builds and tripped the
  local-fast artifact guard.

### Evidence

- Hermetic linked-worktree regression checks no-op reuse, HEAD changes, tag
  packing and subsequent reuse.

### Links

- Refs #2778
- Prerequisite validation fix for #2507
