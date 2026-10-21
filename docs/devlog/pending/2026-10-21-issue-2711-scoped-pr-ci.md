## 2026-10-21 - Stop escalating every PR and queue run to full Linux tests
<!-- devlog-id: 2026-10-21-issue-2711-scoped-pr-ci -->

### Result

- Crates with no workspace dependents (forge-cli, agent-session) are ordinary
  owned crates again, so their PRs and queue groups run the affected scope
  instead of FULL.
- The three audit baselines are audit-only: they select no package and no
  longer escalate. A generated completion asset follows the package that owns
  its binary; unknown binaries and every other scripts path still escalate.
- The semantic-commit registry rule no longer selects nils-common, whose
  reverse closure covered most of the workspace.
- The scoped lane runs the cheap non-compiling audits a crate-only change can
  break, plus completion flag parity for the selected binaries without a
  workspace build. Every PR and queue run writes its scope, packages and
  reasons to the job summary.

### Why / context

- Every recent PR and queue run logged FULL even though the selector worked;
  the hard-escalate list and one registry rule defeated it. The merge queue
  keeps consuming the same plan, and nightly and release validation stay full.

### Evidence

- Replaying the 30 latest main commits through the old and new policy over the
  current workspace graph selected the affected scope for 20 (was 2); the rest
  changed lockfiles, releases or CI scripts and stay FULL.
- New selector, executor, job-summary and parity-audit tests failed before the
  change and pass after it.

### Links

- Issue #2711

### Follow-ups

- Reuse main's cached dependency artifacts for package-scoped builds (item 5 of
  #2711); choose the mechanism by measuring non-workspace compile counts.
