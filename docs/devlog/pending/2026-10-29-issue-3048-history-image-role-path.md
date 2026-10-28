## 2026-10-29 - Correct tool-image history roles and explain local-file refusals
<!-- devlog-id: 2026-10-29-issue-3048-history-image-role-path -->

### Result

- Image-bearing tool results are assistant output; refused local files expose
  an outside-allowed-roots reason without widening the file boundary.

### Why / context

- Tool envelopes are transport records rather than human prompts, and a generic
  unavailable error hid an intentional file-boundary refusal.

### Evidence

- Three new fixture regressions failed before production edits; all seven
  focused image regressions pass after the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3048>

### Follow-ups

- Confirm the affected deployment route status and consumer fallback through
  the tracking issue.
