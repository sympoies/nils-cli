## 2026-11-26 - Recover short Claude wakes after capture loss
<!-- devlog-id: 2026-11-26-issue-4050-mailbox-wake-recovery -->

### Result

- Wait within the guarded terminal budget for a temporarily unreadable Claude
  wake composer before the first Enter.

### Why / context

- A post-paste capture outage could leave a short wake in the composer and mark
  its attempt unknown without submission.

### Evidence

- Synthetic late-paste, capture-loss, runtime replacement and unknown-outcome
  cases exercise the built-in producer.

### Links

- <https://github.com/sympoies/nils-cli/issues/4050>
