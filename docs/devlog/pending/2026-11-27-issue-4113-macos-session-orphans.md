## 2026-11-27 - Stop macOS session orphans during close
<!-- devlog-id: 2026-11-27-issue-4113-macos-session-orphans -->

### Result

- Ordinary macOS close sends TERM, then KILL after a bounded grace period to
  orphaned processes whose environment matches the session and runtime launch
  markers.

### Why / context

- Darwin audit-token signaling binds the target to its exec generation. Shared
  sccache servers carry the session markers and are retained when their
  executable and server environment identify them. A matching native Codex
  0.162.1 PID record also protects a shared app-server or updater from signals.

### Evidence

- A synthetic runtime exits with a detached child. The regression covers the
  TERM-to-KILL sequence, an argv lookalike, a marked shared server, and a
  refusal with the exact surviving PID and one recovery command. The Codex
  fixture preserves a proven shared daemon and stops a marked orphan using
  the same executable when the provider's recorded identity is stale.

### Links

- <https://github.com/sympoies/nils-cli/issues/4113>
