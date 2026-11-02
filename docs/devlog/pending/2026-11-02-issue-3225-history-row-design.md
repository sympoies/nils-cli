## 2026-11-02 - Pin the history row class and notice summary design
<!-- devlog-id: 2026-11-02-issue-3225-history-row-design -->

### Result

- Design-only specifications and a language-neutral schema fixture pin
  runtime_command, compaction and notice rows, shared notice labels, and
  bounded summary input; runtime implementation follows after design review.

### Why / context

- Consumers need one immutable fixture before independent producer and UI work.
  Stored machine text recognition is display-only, while unknown rows keep no
  class; existing mailbox wake bytes remain invariant.

### Evidence

- Inspected the three design issue threads, current provider history and notice
  contracts, and existing compaction and mailbox fixtures. Repository gate
  validation is recorded in the design PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3225>
- <https://github.com/sympoies/nils-cli/issues/3226>
- <https://github.com/sympoies/nils-cli/issues/3228>
