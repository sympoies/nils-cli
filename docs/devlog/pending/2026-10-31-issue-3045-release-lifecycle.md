## 2026-10-31 - Guarded GitHub release lifecycle operations
<!-- devlog-id: 2026-10-31-issue-3045-release-lifecycle -->

### Result

- Added release view, guarded file/stdin notes editing and explicit release
  deletion with independent tag cleanup, canonical SemVer protection and
  verified readback.

### Why / context

- Release operators need exact release/tag identity, actual target commits and
  honest partial outcomes without bypassing repository, actor or payload
  guards.

### Evidence

- Hermetic lifecycle fixtures passed, including retained pre-repair failures
  for concurrent tag movement and published-only REST draft lookup; focused
  crate clippy passed. Full/local-fast validation remains
  delivery-steward-owned.

### Links

- <https://github.com/sympoies/nils-cli/issues/3045>
