## 2026-10-28 - Claude compact history retains one entry per compaction
<!-- devlog-id: 2026-10-28-issue-3014-compact-history -->

### Result

- Provider history suppresses literal compact input with arguments or
  prompt-source metadata and retains each command-markup echo.

### Why / context

- The literal input and command echo can be separated by a compaction boundary
  and summary; text-based deduplication would also lose genuinely repeated
  commands.

### Evidence

- Synthetic fixture failed with two entries for one compaction before the fix.
  Provider-history unit scope passes: 78 passed, 1 ignored, including
  forward/reverse pagination, structured parts and incremental projection.

### Links

- <https://github.com/sympoies/nils-cli/issues/3014>
