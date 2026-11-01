## 2026-11-02 - Spell out coalesced mailbox wake counts on notice delivery
<!-- devlog-id: 2026-11-02-issue-3180-explained-wake-counts -->

### Result

- Notice delivery of coalesced mailbox wakes now names each category with its
  count and ends with the inbox action, for example `Mailbox: 1 blocker, 1
  progress, 3 report; agent-session message inbox`. The compact letters remain
  only on the typed composer reminder, where the 51-column one-row bound
  applies.

### Why / context

- Refs #3180 and #3186: the compact letters were unexplained to agents and
  people. A notice envelope is multiline and is not held to the one-row width
  bound, whichever route delivers it.

### Evidence

- RED before the change: six notice-delivery tests in the serve mailbox notice
  suite failed with the compact body against the spelled-out expectation, while
  the gate-off plain route passed.
- GREEN after the change: the same suite and the coalescing and notification
  unit tests pass; the compact recognizers and the plain route are unchanged.

### Follow-ups

- Fleet step: a consumer that translates the compact letters should drop that
  translation. Reinstall agent-session to pick up the new body; no old-form
  alias is kept.
