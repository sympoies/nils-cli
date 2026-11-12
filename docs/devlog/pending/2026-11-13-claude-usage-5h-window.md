## 2026-11-13 - Fix claude-cli usage 5h window parsing
<!-- devlog-id: 2026-11-13-claude-usage-5h-window -->

### Result

- `claude-cli usage --source cli` now reads only the `Current session` and
  `Current week (all models)` sections of the `/usage` screen; a window it
  cannot parse is reported unavailable instead of a wrong value.

### Why / context

- The loose `session`/`5h` keyword match picked up later screen text (for
  example `sessions on this machine only`, `15h ago`) and overwrote the 5-hour
  window with an unrelated percent and reset string.

### Evidence

- Fixture `crates/claude-cli/tests/fixtures/claude_usage_screen.txt` is a
  sanitized capture of the Claude Code 2.1.317 layout; retired-layout test
  inputs were replaced.
