## 2026-11-03 - Classify unmarked history notices
<!-- devlog-id: 2026-11-03-issue-3226-history-notices -->

### Result

- Recognize known notice forms consistently in history and card previews
  without changing stored text.

### Why / context

- Historical provider rows and fallback notices lack an envelope; recognition
  shares the existing label table and leaves private bootstrap text
  unclassified.

### Evidence

- Synthetic HTTP history fixture and paging tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3226>
