## 2026-11-17 - Allow stale ancestor review dismissal
<!-- devlog-id: 2026-11-17-stale-review-dismiss -->

### Result

- Added an explicit opt-in for dismissing an automated review on an ancestor of
  the current head.

### Why / context

- A designated pass can address a review from an earlier commit while retaining
  the existing dismissal guards.

### Evidence

- Synthetic provider fixtures cover ancestor admission and unrelated commit
  refusal.

### Links

- <https://github.com/sympoies/nils-cli/issues/3343>
