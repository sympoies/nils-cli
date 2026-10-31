## 2026-11-01 - Expose provider-reported aggregate output overruns
<!-- devlog-id: 2026-11-01-issue-3161-output-overrun -->

### Result

- Research results now preserve usage and return an explicit partial failure
  when Anthropic reports aggregate output above the caller token limit.

### Why / context

- A provider generation parameter does not prove a hard aggregate billing cap;
  already generated upstream tokens cannot be retroactively capped.

### Evidence

- The focused overrun acceptance case failed on the original implementation,
  then the 20-test package suite passed after the finite repair. No additional
  live call was made for the repair.

### Links

- <https://github.com/sympoies/nils-cli/pull/3165>
