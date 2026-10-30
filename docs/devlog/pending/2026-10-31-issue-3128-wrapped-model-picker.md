## 2026-10-31 - Wrapped Claude model picker confirms owner commands
<!-- devlog-id: 2026-10-31-issue-3128-wrapped-model-picker -->

### Result

- Owner command-UI recognition accepts Claude's Select model picker when a
  narrow pane wraps its option rows and leaves a final '· Esc to cancel' footer
  row, so /model returns command_dispatched instead of unknown

### Why / context

- The recognizer only accepted a bare or ' · Esc to cancel'-suffixed footer; at
  56 columns the hint wraps and the last row starts with the separator

### Evidence

- Synthetic fixture of original structural rows from Claude Code 2.1.296 at 56
  columns; the HTTP owner-boundary scenario was RED (409 unknown) before the
  footer rule changed and passes after; the 80-column and negative scenarios
  are unchanged

### Links

- <https://github.com/sympoies/nils-cli/issues/3128>
