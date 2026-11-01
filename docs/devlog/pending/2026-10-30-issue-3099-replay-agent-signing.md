## 2026-10-30 - Authenticate signing for managed branch replay
<!-- devlog-id: 2026-10-30-issue-3099-replay-agent-signing -->

### Result

- Added the finite semantic-commit rebase and continuation owner, preserving
  original authors while authenticating the selected committer and signer for
  each invocation.

### Why / context

- Detached continuation must retain the original branch remote precedence; raw
  Git replay cannot supply the managed forge identity when the ambient signing
  key is unavailable.

### Evidence

- Pre-production raw signing and source-built owner regressions were captured;
  focused replay and continuation tests verify good selected-key signatures,
  original authors, resulting bytes and parents, authorization refusal and
  staged resolution retention.

### Links

- <https://github.com/sympoies/nils-cli/issues/3099>
