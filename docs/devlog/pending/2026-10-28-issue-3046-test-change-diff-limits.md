## 2026-10-28 - Bound review test-change inspection to source patches
<!-- devlog-id: 2026-10-28-issue-3046-test-change-diff-limits -->

### Result

- Ordinary test-change previews enumerate complete metadata and capture only
  supported source patches; large asset cleanups no longer consume patch
  limits. Oversized source patches name affected paths.

### Why / context

- A full raw PR diff could exceed the test-change bound before unrelated
  evidence deletions were excluded, blocking designated review.

### Evidence

- Regression first failed on more than 4 MiB of deleted evidence plus four
  small tests; focused unit and CLI integration validation cover the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3046>
