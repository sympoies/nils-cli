## 2026-10-27 - Coalesce coordinator mailbox wakes per destination
<!-- devlog-id: 2026-10-27-issue-3008-coordinator-mailbox -->

### Result

- Add per-recipient 600-second routine notification windows, bounded category
  counts and urgent/service bypass.

### Why / context

- Routine coordination traffic should not repeatedly interrupt a busy
  coordinator.

### Evidence

- Destination admission regression failed before implementation; focused policy
  tests now pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3008>
