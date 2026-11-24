## 2026-11-24 - Bound Codex notice and coordination registry poll reads
<!-- devlog-id: 2026-11-24-issue-3979-remaining-serve-poll-reads -->

### Result

- Reuse validated registry generations across serve polls and resume Codex
  notice scans at the last complete row when the rollout grows continuously.

### Why / context

- An unresolved mailbox attempt repeatedly scanned up to 64 MiB of an unchanged
  rollout even with no clients. Session list polls and idle discovery also
  reloaded the complete registry.

### Evidence

- A private daemon with six polling clients attributed one complete registry
  read to each warmed session list poll. Both read-budget regressions failed
  before the fix and passed in the focused 91-test run.

### Links

- <https://github.com/sympoies/nils-cli/issues/3979#issuecomment-7118492337>
