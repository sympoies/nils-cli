## 2026-10-30 - Prove Claude wrapped pastes with omitted trailing spaces
<!-- devlog-id: 2026-10-30-issue-2979-claude-trailing-space-proof -->

### Result

- Known wrapped Claude pastes retain their logical trailing-space cursor cells
  and use the true-end proof before one submit.

### Why / context

- A retained synthetic 315-character input timed out because final-space
  reconstruction and true-end verification were Codex-only; padding-normalized
  Claude text also failed exact known-paste comparison.

### Evidence

- Pre-production CLI regression reproduced the timeout; repaired synthetic
  ASCII and CJK cases submit once with exact modeled history and idempotency
  refusal, and hidden appended spaces prevent Enter.

### Links

- <https://github.com/sympoies/nils-cli/issues/2979>
