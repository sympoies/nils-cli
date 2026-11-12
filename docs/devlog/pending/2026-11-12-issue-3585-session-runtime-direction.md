## 2026-11-12 - Record the hybrid session runtime direction
<!-- devlog-id: 2026-11-12-issue-3585-session-runtime-direction -->

### Result

- Added the session runtime v1 direction record: structured sources by default,
  real terminal takeover kept, an explicit single-writer rule, and a
  per-provider signal map. Codex uses the app-server with a remote TUI. For
  Claude, TUI in tmux with official structured sources is proposed, pending
  maintainer decision.

### Why / context

- The spike measured that Claude has no supported live attach to an SDK or
  stream-json session and no session lock, so the runtime must own the
  single-writer rule and keep the terminal view.

### Evidence

- Spike report on the tracking issue; docs-only local checks.

### Links

- <https://github.com/sympoies/nils-cli/issues/3585>
