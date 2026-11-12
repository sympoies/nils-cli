## 2026-11-12 - Read Claude session state from claude agents --json
<!-- devlog-id: 2026-11-12-issue-3590-claude-agents-state-source -->

### Result

- The shadow sampler now reads the Claude provider state (idle, busy, waiting
  with reason) from one cached `claude agents --json` call per config directory
  and ranks it ahead of screen rules. The observation records its source, and
  an unavailable source falls through to the screen rules and says why. Screen
  rules stay as the fallback.

### Why / context

- The session runtime direction (#3585, option C) replaces screen heuristics
  with official structured sources; this is the first provider-owned state
  source for Claude.

### Evidence

- Sanitized fixture tests for idle, busy, waiting, unmatched, unknown-status
  and malformed output; sampler tests for source precedence, fallthrough
  reasons, one call per config directory and an explicit child environment.

### Links

- <https://github.com/sympoies/nils-cli/issues/3590>

### Follow-ups

- Remove the screen rules the structured source replaces (#3592).
