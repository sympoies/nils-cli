## 2026-11-25 - Report unobservable retained pastes to patrol
<!-- devlog-id: 2026-11-25-unobservable-retained-paste -->

### Result

- Flag a retained unsubmitted paste when its composer cannot be observed.

### Why / context

- A capture loss after paste could hide a possibly stranded attempt.

### Evidence

- The capture-loss regression failed on the baseline and passed after the fix.
- The focused long-paste suite passed all nine tests; the regression passed the
  tempdir leak probe with no leftovers.
- The local-fast package gate passed 3,456 tests after correcting runtime and
  temp-directory setup errors from earlier attempts.

### Links

- <https://github.com/sympoies/nils-cli/issues/4052>
