## 2026-11-26 - Preserve closed coordinator evidence for open review handoffs
<!-- devlog-id: 2026-11-26-issue-3972-review-handoff-retention -->

### Result

- The authenticated closed feed and local recovery view preserve existing
  terminal rows referenced by the operator dependency snapshot.

### Why / context

- Review recovery needs the original terminal timestamp and serving machine
  after ordinary board retention ends.

### Evidence

- A synthetic pruning regression reproduced the missing coordinator row before
  the fix. The ledger tests cover selective retention and release.

### Links

- <https://github.com/sympoies/nils-cli/issues/3972>

### Follow-ups

- Snapshot publication and federated board consumption are tracked in
  <https://github.com/sympoies/nils-cli/issues/4091>.
