## 2026-11-25 - Wait for guarded long pastes before submitting
<!-- devlog-id: 2026-11-25-agent-session-guarded-long-paste -->

### Result

- Guarded Codex and Claude terminal sends wait for the first paste to render
  before verifying and submitting it.
- Keyed attempt status reports retained wrapped input and recovery verifies the
  original digest and logical composer before sending one Enter.

### Why / context

- A delayed long paste could land in the composer after the submit guard had
  already refused.

### Evidence

- Synthetic Codex and Claude terminal and Codex app-server probes, followed by
  guarded-send regression tests.

### Links

- [Issue #4025](https://github.com/sympoies/nils-cli/issues/4025)
