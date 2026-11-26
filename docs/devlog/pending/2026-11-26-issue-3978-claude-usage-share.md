## 2026-11-26 - Share Claude usage refreshes and honor account cooldowns
<!-- devlog-id: 2026-11-26-issue-3978-claude-usage-share -->

### Result

- Concurrent Claude usage readers share one refresh. No read refreshes inside
  an unavailable-source retry interval or while every account is in its
  rate-limit cooldown. Rate-limited accounts report backoff_until.

### Why / context

- Forced and concurrent reads queued duplicate helper runs and kept querying
  after a 429.

### Evidence

- Mock-transport endpoint tests were red on main for duplicate refresh, refresh
  inside the 429 deadline, and repeated unavailable-source reads; they pass
  after the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3978>

### Follow-ups

- Wiring the claude-cli prompt segment and diag rate-limits consumers to the
  shared endpoint remains on issue 3978.
