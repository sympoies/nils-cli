## 2026-11-02 - Separate command executable names from schema identity
<!-- devlog-id: 2026-11-02-agent-session-command-identity -->

### Result

- Agent-facing command hints outside mailbox reminder owners use the shared
  build-time executable name; all schema identifiers keep the fixed
  agent-session namespace.

### Why / context

- Renamed builds need recovery commands that agree with the shared session-binary
  lookup owner. This reuses its existing build-time setting.
- Mailbox reminder text and read-back stay with their active owners and will be
  converted in part B.

### Evidence

- The renamed executable test failed before production edits: help examples
  retained the default name. A configured-build regression also failed before
  reusing the shared name owner.
- Source inventory preserves all 456 schema ID literals.

### Links

- <https://github.com/sympoies/nils-cli/issues/3193>
