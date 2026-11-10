## 2026-11-10 - Prioritize managed Claude History inside the catalog budget
<!-- devlog-id: 2026-11-10-issue-3504-history-priority -->

### Result

- Ordinary History seeds validated registered Claude transcripts before the
  broad walk, retaining the shared two-second and 10,000-entry bounds.

### Why / context

- A preceding Codex source could consume the deadline before any Claude
  transcript was inspected. Sorting discovered paths alone cannot recover a
  later source; registry IDs allow bounded named-file probes first.

### Evidence

- The over-budget fresh-session regression failed on main before production
  edits and passed after the fix; the affected History suite passed 96 tests
  with one existing ignored case.

### Links

- <https://github.com/sympoies/nils-cli/issues/3504>
