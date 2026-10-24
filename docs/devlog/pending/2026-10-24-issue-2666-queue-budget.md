## 2026-10-24 - Extend merge queue observation beyond the declared check window
<!-- devlog-id: 2026-10-24-issue-2666-queue-budget -->

### Result

- The default merge queue observation budget is 5700 seconds, with explicit
  overrides retained and queued or unknown timeout outcomes accompanied by
  scoped read commands.

### Why / context

- The sequential compile/test queue workflow declares a 90-minute check window;
  the old 45-minute observer could expire first. Five minutes of polling margin
  avoids promising that an unobserved queue transition will merge.

### Evidence

- RED captured the old default and misleading timeout recovery; GREEN passed 14
  queue policy tests, 79 merge CLI tests and 51 delivery CLI tests. Package
  Clippy and formatting checks passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2666>
