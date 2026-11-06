## 2026-11-06 - Recipient mailbox quota signals
<!-- devlog-id: 2026-11-06-issue-3415-mailbox-quota-signals -->

### Result

- Quota refusals retain bounded body-free destination evidence, and threshold
  crossings queue a fixed mailbox alert through the existing guarded
  notification path.

### Why / context

- A full mailbox containing read but unacknowledged messages must remain
  observable without consuming another message slot or changing quota and
  acknowledgement semantics.

### Evidence

- Verified the missing rejection event on unchanged production code, then
  passed five fixture mailbox integration tests covering rejection privacy,
  default and configured thresholds, byte usage, and re-arming.

### Links

- <https://github.com/sympoies/nils-cli/issues/3415>
