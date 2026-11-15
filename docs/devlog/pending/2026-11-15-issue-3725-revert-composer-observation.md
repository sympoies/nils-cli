## 2026-11-15 - Revert guarded composer observation reasons
<!-- devlog-id: 2026-11-15-issue-3725-revert-composer-observation -->

### Result

- Reverted the guarded composer observation changes from PR 3721.

### Why / context

- A timing-sensitive test could exceed the guard budget on loaded runners and
  block the merge queue.

### Evidence

- Reverted the merged change; focused guarded send and documentation checks are
  required before delivery.

### Links

- <https://github.com/sympoies/nils-cli/issues/3725>
