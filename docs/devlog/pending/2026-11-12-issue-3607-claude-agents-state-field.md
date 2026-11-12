## 2026-11-12 - Read the Claude state field and pin call bounds
<!-- devlog-id: 2026-11-12-issue-3607-claude-agents-state-field -->

### Result

- The Claude structured state source reads the provider state field (working,
  blocked, done, failed, stopped) when present and uses status only when it is
  absent; an unknown state falls through to the screen rules with a recorded
  reason. The cache TTL, cache key, provider filter, Codex exclusion, call
  timeout and output cap are pinned by tests.

### Why / context

- The provider documents a state field and status then only reports whether the
  process is busy, so a status-only reader could read a working session as
  idle. Refs #3607, follow-up to #3590.

### Evidence

- Tests for both entry shapes, unknown and non-string state, a three-entry
  conflicting duplicate list, and one mutation per bound; supported version
  window is stated in the turn-state contract.

### Links

- <https://github.com/sympoies/nils-cli/issues/3607>

### Follow-ups

- Remove the screen rules the structured source replaces (#3592).
