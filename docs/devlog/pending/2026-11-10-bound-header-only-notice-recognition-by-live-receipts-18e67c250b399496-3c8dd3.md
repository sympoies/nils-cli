## 2026-11-10 - Bound header-only notice recognition by live receipts
<!-- devlog-id: 2026-11-10-bound-header-only-notice-recognition-by-live-receipts-18e67c250b399496-3c8dd3 -->

### Result

- History treats unclaimed notice headers after a session receipt as user
  content while preserving earlier history.

### Why / context

- A provider-session receipt establishes the cutoff without changing historical
  rows for sessions with no live receipt.

### Evidence

- The focused regression failed before the cutoff and passed after it; native
  notice paging tests passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3482>
