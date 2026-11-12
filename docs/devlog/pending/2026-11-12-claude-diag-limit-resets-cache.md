## 2026-11-12 - claude-cli diag rate-limits keeps the last limit_resets on cached results
<!-- devlog-id: 2026-11-12-claude-diag-limit-resets-cache -->

### Result

- `claude-cli diag rate-limits` stores the last network `limit_resets` with the
  rate-limit cache entry; cached and cache-fallback results carry it with an
  additive `limit_resets_checked_at` (epoch seconds of the read that supplied
  it), and network results carry the same field.

### Why / context

- A reader that keeps the latest ok result lost reset data whenever a profile
  fell back to the cache, for example under a usage-endpoint rate limit; an
  entry with no stored value still omits both fields.

### Evidence

- Integration tests in
  `crates/claude-cli/tests/integration/diag_rate_limits.rs` cover cache
  fallback, `--cached`, and no prior read; contract in
  `crates/claude-cli/docs/specs/claude-cli-json-contract-v1.md`; the claude-cli,
  codex-cli and nils-common test suites pass.
