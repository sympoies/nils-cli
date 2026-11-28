## 2026-11-29 - Share one Python runner per opted-in hook dispatch
<!-- devlog-id: 2026-11-29-issue-4214-python-event-runner -->

### Result

- Agent-hook can share a framed runtime-kit Python runner across a provider
  event, including coordination after the existing aggregate decision.

### Why / context

- A Bash or Write event can select several Python handlers. Sharing an
  interpreter avoids their repeated startup and imports.

### Evidence

- Synthetic dispatcher tests compare standalone and batched provider decisions,
  handler order, process identity, deadline recovery, and output budgets.

### Links

- <https://github.com/sympoies/nils-cli/issues/4214>
- <https://github.com/sympoies/agent-runtime-kit/pull/453>
