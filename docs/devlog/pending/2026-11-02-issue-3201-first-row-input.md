## 2026-11-02 - Read a Claude input row that is the first row of the capture
<!-- devlog-id: 2026-11-02-issue-3201-first-row-input -->

### Result

- A Claude input row on the first row of the capture, with its one closing rule
  directly below, is now read as usual. Before, the missing opening rule meant
  no box was found and a guarded send refused with send-composer-unobservable.
  A first-row draft is read as a draft, and a choice, a multi-row input, a
  dialog footer or any other rule layout stays unobservable.

### Why / context

- Refs 3201 and 3186: a real refused pane was run through the reader offline
  and the failed step was composer-not-located, because the input row was the
  first row of the capture and no rule opens the box above it. The pane was 8
  rows high, and the rows above the input row hold the banner, not a rule.

### Evidence

- RED before the change: the first-row empty and draft tests and the derived
  corpus entry failed; the refusal controls passed. A first full gate then
  failed the command-UI test: a first-row input with an autocomplete list below
  its rule was read as a composer and received a second Enter. A list row after
  the rule now keeps the pane unreadable, with its own RED. GREEN after: all
  focused tests pass.
- One existing assertion changed: the retained dismissed-picker frame omits its
  top rule, so its outcome was unknown only because the pane was unreadable.
  It is now submitted, as the same frame with its top rule already was.

### Follow-ups

- This fixes the small-pane shape only. Whether the original report was this
  shape is unknown: one sample on one provider build. A native check on an
  installed build is still needed before 3201 can close.
