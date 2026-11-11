## 2026-11-11 - claude-cli diag rate-limits reports weekly_scoped windows
<!-- devlog-id: 2026-11-11-claude-diag-weekly-scoped-windows -->

### Result

- `claude-cli diag rate-limits` adds one `weekly_scoped:<model>` window per
  `weekly_scoped` entry of the usage `limits[]`, with additive `kind`, `model`
  and `allowance_percent_of_weekly` fields; cached results keep them.

### Why / context

- A per-model meter needs per-model weekly usage and its share of the weekly
  allowance; the shared window type gained the three optional fields, so
  existing windows and Codex output are unchanged.

### Evidence

- Fixture-driven integration tests in `crates/claude-cli/tests/integration/diag_rate_limits.rs`;
  contract in `crates/claude-cli/docs/specs/claude-cli-json-contract-v1.md`;
  `--local-fast` checks pass.
