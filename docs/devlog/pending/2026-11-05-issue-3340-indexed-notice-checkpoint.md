## 2026-11-05 - Bound native notice archive lookups with authenticated checkpoints
<!-- devlog-id: 2026-11-05-issue-3340-indexed-notice-checkpoint -->

### Result

- Automatic first-writer import preserves original archives and builds
  authenticated reservation/physical-row indexes; full provenance verification
  remains available through notice-store audit.

### Why / context

- A metadata sweep still takes 458 ms at 150003 small-generation files. Normal
  calls authenticate selected paths; the explicit full audit detects corruption
  of unvisited originals. Independent durable flushes overlap without removing
  publication barriers.

### Evidence

- Bounded-read RED preceded production edits; 13 focused lifecycle cases
  passed. Explicit optimized 10k/100k/1M runs in both generation shapes passed:
  ordinary admission below 15 ms, History below 16 ms and rotation below 425
  ms.

### Links

- <https://github.com/sympoies/nils-cli/issues/3340>
