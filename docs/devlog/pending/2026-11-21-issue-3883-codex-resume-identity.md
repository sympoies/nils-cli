## 2026-11-21 - Retain the managed Codex resume identity through normal exit
<!-- devlog-id: 2026-11-21-issue-3883-codex-resume-identity -->

### Result

- The managed Codex prompt dispatcher passes the exact conversation ID
  privately to the session helper after activity admission. The helper checks
  the runtime and projected identity before writing resume metadata.

### Why / context

- Turn activity can reach completed with a projected identifier while the exact
  conversation ID is still unavailable for stopped resume. When multiple
  rollouts fit the launch, history discovery cannot supply that ID safely.

### Evidence

- A synthetic managed start, completed turn, zero-status exit, and exact-thread
  relaunch fixture reproduced session-not-resumable before the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3883>
