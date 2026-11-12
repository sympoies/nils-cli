## 2026-11-04 - Hold the wake budget in the stale-draft wake test
<!-- devlog-id: 2026-11-04-issue-3281-stale-draft-wake-budget -->

### Result

- stale_draft_is_saved_before_wake_for_both_providers holds HoldBudgetGuard
  like its sibling wake tests, so the shortened unit-test hold budget cannot
  expire during its two starts per provider on a slow runner.

### Why / context

- The 2.5 s unit-test hold budget failed this test on slow macOS runners;
  pinned to one CPU beside one busy loop it failed 6 of 6 runs, and with the
  budget widened it passed 6 of 6 under one to three loops.

### Evidence

- Same test, one busy loop on the same CPU: 0 of 6 before, 6 of 6 after; 30 of
  30 uncontended runs and the 45-test claude_mailbox group passed through the
  gate wrapper.

### Links

- <https://github.com/sympoies/nils-cli/issues/3281>
