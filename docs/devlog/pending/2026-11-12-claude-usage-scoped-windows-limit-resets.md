## 2026-11-12 - claude-cli usage reports scoped weekly windows and limit resets
<!-- devlog-id: 2026-11-12-claude-usage-scoped-windows-limit-resets -->

### Result

- claude-cli usage adds weekly_scoped:<model> windows and a limit_resets
  section (session program state, one entry per grant with kind from clears) to
  text and claude-cli.usage.v1 JSON, additively.
- The usage OAuth read now uses the diag status client identity so grants are
  returned; scoped windows and resets reuse the diag parsers.

### Why / context

- The usage response exposes per-model weekly windows and granted resets that
  the usage command dropped (#3548).

### Evidence

- crates/claude-cli/tests/integration/usage_limits.rs fixtures for an
  individual and a team plan; docs/specs section Usage scoped windows and limit
  resets.

### Follow-ups

- The CLI-scrape fallback still omits scoped windows; documented in the
  contract.
