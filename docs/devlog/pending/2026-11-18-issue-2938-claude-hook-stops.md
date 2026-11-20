## 2026-11-18 - Document Claude hook-induced turn stops
<!-- devlog-id: 2026-11-18-issue-2938-claude-hook-stops -->

### Result

- Document lifecycle evidence and the supported consumer strategy for Claude
  PostToolUse stops. Add a modeled callback regression.

### Why / context

- A parallel native lifecycle hook receives the tool result without the
  stopping hook decision.

### Evidence

- Claude Code hook reference and the agent-session completion contract.

### Links

- <https://github.com/sympoies/nils-cli/issues/2938>
