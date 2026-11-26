## 2026-11-26 - Regenerate release folds when main advances
<!-- devlog-id: 2026-11-26-issue-4070-release-fold-regeneration -->

### Result

- Release fold admission regenerates stale preparations from protected main and
  verifies the replacement tree, App signature, and fresh exact CI before
  enqueue.

### Why / context

- An ordinary merge between preparation and admission made the retained release
  fold permanently stale on rerun.

### Evidence

- The moved-main fixture failed on the old admission gate and now checks that
  only the replacement enters the queue after fresh CI. Exact fold policy
  fixtures continue to reject history edits and fragment changes.

### Links

- [Issue #4070](https://github.com/sympoies/nils-cli/issues/4070)
