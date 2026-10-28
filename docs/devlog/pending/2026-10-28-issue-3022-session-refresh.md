## 2026-10-28 - agent-session on-demand refresh of model, effort and title
<!-- devlog-id: 2026-10-28-issue-3022-session-refresh -->

### Result

- Added agent-session refresh ID and POST /sessions/{id}/refresh: re-read the
  model and Codex effort from the provider transcript, regenerate a structured
  unpinned title from current work issues, and return the changed fields plus
  the refreshed session view.

### Why / context

- Launch-time model and effort go stale after a manual provider change, and the
  account brokers expose no signed-in identity, so the provider account is
  reported as unavailable instead of guessed.

### Evidence

- Focused lib tests for the refresh module and serve route, a real-CLI
  integration test, file-size audit and local-fast checks.

### Links

- <https://github.com/sympoies/nils-cli/issues/3022>
