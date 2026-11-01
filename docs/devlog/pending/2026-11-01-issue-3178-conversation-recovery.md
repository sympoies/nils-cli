## 2026-11-01 - Restore conversation context after event degradation
<!-- devlog-id: 2026-11-01-issue-3178-conversation-recovery -->

### Result

- Both conversation degradation producers now scope their note to the current
  event; a successful evaluation of the previously degraded checks emits one
  recovery context for the exact product/session/runtime.
- Recovery matches the affected selected rule and capability settings, and
  remains pending when trace preparation fails before provider delivery.
- A failed presentation-state clear preserves the original admission decision
  without emitting recovery, leaving it pending for a later successful event.

### Why / context

- A transient fault can leave text in a long-running conversation after the
  fault clears. Private presentation state tracks the observed checks without
  granting mutation authority or treating another subsystem, missing rules or
  elapsed time as recovery.

### Evidence

- Before production edits, four process-level regressions failed on missing
  recovery reasons or provider context. The same four tests now pass, covering
  both note producers, repeated healthy events, identity isolation and
  unchanged failed-mutation refusal.
- Three additional process regressions reproduced review findings on the
  reviewed production head: unrelated coordination success, a reused rule ID
  with a changed probe, and recovery lost after a failed trace. All seven
  process regressions pass after the finite repair.
- A further real process regression reproduced recovery emitted after a failed
  state-clear write. With writes restored, recovery is delivered once and then
  omitted; all eight process regressions pass after the write-failure repair.

### Links

- <https://github.com/sympoies/nils-cli/issues/3178>

### Follow-ups

- Designated review, current-head CI, containing release and original
  acceptance remain separate delivery gates.
