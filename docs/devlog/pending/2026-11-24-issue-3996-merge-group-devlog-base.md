## 2026-11-24 - Attribute merge-group devlog changes to the queue base
<!-- devlog-id: 2026-11-24-issue-3996-merge-group-devlog-base -->

### Result

- Merge-group devlog checks compare the checked-out candidate with the
  immutable base from the queue event.

### Why / context

- Full validation could compare a version-only release against a stale remote
  ref and attribute an earlier trusted fold to the release.

### Evidence

- The devlog policy fixtures reproduce the stale-ref failure and verify that a
  release behind a fold and a canonical fold pass while ordinary fragment
  deletions and month edits fail.

### Links

- <https://github.com/sympoies/nils-cli/issues/3996>
