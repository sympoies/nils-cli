## 2026-11-04 - A version-like idle suggestion no longer reads as a selection list
<!-- devlog-id: 2026-11-04-issue-3149-version-like-suggestion-is-not-a-choice -->

### Result

- A Claude idle suggestion that starts like a version, such as digit period
  digit, was taken for the cursor row of a selection list, so the composer was
  not located and every direct reply was refused until the owner typed. A
  numbered choice is now digits, a period, then whitespace or the end of the
  row.
- Dim or colored rows such as a numbered option stay choices, so the pinned
  rule that a dim numbered row is never an empty composer holds. The same
  function serves Codex.

### Why / context

- Refs 3149: read-only styled captures of an idle pane, run through the guarded
  reader, returned composer-not-located exactly for the captures that show the
  suggestion and read every other capture correctly. Typing replaced the
  suggestion, which is why one owner input restored delivery until the next
  idle suggestion.

### Evidence

- A reader test and a direct prompt boundary test: a version-like dim
  suggestion delivers on both routes; numbered rows, dim or not, stay refused
  without pane text.

### Follow-ups

- The stranded paste placeholder is tracked separately in 3309.
