## 2026-11-22 - Bound provider continuity epochs to safe integers
<!-- devlog-id: 2026-11-22-issue-3877-bound-continuity-epochs -->

### Result

- Daemon snapshots omit provider continuity proofs containing an epoch above
  9007199254740991.

### Why / context

- The relay consumes continuity epochs as JavaScript safe integers.

### Evidence

- The pre-fix snapshot contract test reproduced publication of the first unsafe
  epoch.

### Links

- <https://github.com/sympoies/nils-cli/issues/3877>
