## 2026-11-25 - Verify the final Actions gate after a macOS retry
<!-- devlog-id: 2026-11-25-issue-4021-queue-validation-retry-proof -->

### Result

- The required Linux merge-group check, coverage badge, and nightly report
  verify the successful final gate for the current run, attempt, and event
  commit.

### Why / context

- After the initial macOS partitions were cancelled and their retries passed,
  the final gate set its commit output but the caller received it empty.

### Evidence

- Replaying run 45411646819 reproduces the old caller failure and validates the
  exact commit on the new path. Missing, duplicate, stale, and unsuccessful
  gate records are rejected.
- A scheduled and manual retry fixture exercises the badge and tracker with
  successful and failed final gates.

### Links

- <https://github.com/sympoies/nils-cli/issues/4021>
