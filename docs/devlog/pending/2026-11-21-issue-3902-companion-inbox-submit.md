## 2026-11-21 - Submit companion inbox messages through the daemon
<!-- devlog-id: 2026-11-21-issue-3902-companion-inbox-submit -->

### Result

- Added a managed-session command that submits one companion inbox message and
  returns the relay receipt ID.

### Why / context

- The daemon holds the outbound bearer and derives the exact source from the
  current session capability. A missing receipt is ambiguous, so both hops send
  once.

### Evidence

- The synthetic CLI-to-daemon-to-edge fixture reproduces the missing command on
  the baseline and passes all six companion tests with the implementation.

### Links

- <https://github.com/sympoies/nils-cli/issues/3902>
