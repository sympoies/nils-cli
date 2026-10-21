## 2026-10-21 - Run release version bumps on the reduced lane without a base-CI receipt
<!-- devlog-id: 2026-10-21-issue-2711-release-only-lane -->

### Result

- A canonical release version bump now takes the reduced release-only lane
  (version lockstep, third-party artifacts, publish order and a locked cargo
  check, no test suites) as soon as the exact-base detector proves the diff;
  the base-CI receipt wait is removed.
- Any other dependency change still goes through the planner, and the release
  workflow still validates the tagged commit in full.

### Why / context

- The reduced lane required the base commit to carry a full-validation marker.
  Scoped merge-queue runs carry none, so the proof failed and release bump PRs
  ran the full gate for about 27 minutes.

### Evidence

- New workflow-contract tests failed against the old finalize step and the
  proof step, and pass after the change; the release, snapshot, detector and
  planner suites pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2711>
