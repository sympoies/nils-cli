## 2026-11-19 - Restore Claude notice receiver after a provider session cycle
<!-- devlog-id: 2026-11-19-restore-claude-notice-receiver-after-a-provider-session-cycle-18e933ae0062c58b-20b8c2 -->

### Result

- A mounted Claude receiver can renew readiness after an interactive terminal
  start following provider session end.

### Why / context

- The earlier module canceled its timer on session end and then refused to
  start the receiver again.

### Evidence

- The synthetic provider reproduced lost readiness before the change and passed
  after it. The focused native notice suite passed 24 tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3821>
