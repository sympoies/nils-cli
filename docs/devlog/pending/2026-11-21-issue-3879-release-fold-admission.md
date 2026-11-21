## 2026-11-21 - Deliver canonical devlog folds after stable release publication
<!-- devlog-id: 2026-11-21-issue-3879-release-fold-admission -->

### Result

- The Release workflow prepares one signed canonical fold PR and submits its
  verified head to the protected squash queue after required CI passes.

### Why / context

- GitHub Release publication through the workflow token did not trigger the
  former release event listener.

### Evidence

- The fold fixtures cover trusted publication, exact replay, canonical Test
  changes, holds, failed required CI, queue eviction and merged fragment
  read-back.

### Links

- <https://github.com/sympoies/nils-cli/issues/3879>
- <https://github.com/sympoies/nils-cli/issues/3863>
