## 2026-10-31 - Current deleted-coordinator producer recovery acceptance
<!-- devlog-id: 2026-10-31-issue-3021-deleted-producer-acceptance -->

### Result

- An isolated integration case exercises the current delete producer, unchanged
  retained board row, fresh refusal, elapsed staleness, supervisor recovery and
  exact-scope reviewer reassignment.

### Why / context

- The integrated baseline already passed after the released timestamp and
  prerequisite fixes; acceptance adds evidence without changing producer
  behavior or historical recovery trust.

### Evidence

- The real producer-to-consumer probe and 61-second recovery case passed; the
  baseline also passed 20 retired-coordinator cases, three producer close-row
  cases and the CLI-delete retained-row case.

### Links

- <https://github.com/sympoies/nils-cli/issues/3021>
