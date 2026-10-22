## 2026-10-22 - Unify missing completion uncertainty and verify Codex attention
<!-- devlog-id: 2026-10-22-unify-missing-completion-uncertainty-and-verify-codex-attention-18e0c99767cbecfb-e39d1 -->

### Result

- Project pending completion evidence to read-only unknown after a bounded
  quiet window for every harness; expose content-free Claude Stop candidate
  reasons.
- Admit Codex 0.162.1 exact protocol attention and ignore explicitly
  nonblocking user-input requests.

### Why / context

- Missing completion evidence must remain distinct from both working and
  waiting for human input.

### Evidence

- Hermetic regression fixtures reproduce the prior missing uncertainty, absent
  Stop diagnostics and version gate; protocol fixtures protect typed request
  resolution and content exclusion.

### Links

- Refs #2507
