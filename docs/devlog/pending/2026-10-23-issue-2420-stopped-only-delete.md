## 2026-10-23 - Stopped-only session deletion contract
<!-- devlog-id: 2026-10-23-issue-2420-stopped-only-delete -->

### Result

- Added an opt-in agent-session delete mode that requires an expected runtime
  incarnation, proves the session is stopped and quiescent, and removes session
  state without signaling the runtime.
- Blocked stopped-only deletion while durable tmux termination recovery is
  pending, rejected combining the mode with --leave-processes, and documented
  its contract and typed refusals.

### Why / context

- The conditional deletion path must fail closed when runtime identity or
  lifecycle recovery ownership is uncertain; existing deletion modes retain
  distinct semantics.

### Evidence

- The new CLI conflict and pending-termination integration tests failed before
  the fixes and passed afterward. Focused stopped-only integration tests, CLI
  help contract tests, formatting, and the strict file-size audit passed.

### Links

- Issue: <https://github.com/sympoies/nils-cli/issues/2420>
- Primary PR: <https://github.com/sympoies/nils-cli/pull/2830>
