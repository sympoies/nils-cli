## 2026-10-30 - Claude model and effort pickers confirm owner commands
<!-- devlog-id: 2026-10-30-issue-3062-claude-model-effort-ui -->

### Result

- Owner command-UI recognition now accepts Claude's Select model and Effort
  pickers (rule, title, cancellation footer as the last row, no rule or
  composer row below, numbered options only) besides the Settings frame, so
  /model and /effort return command_dispatched instead of unknown
- Codex /model stays unobserved and unrecognized; Codex /effort is a
  native-unsupported draft

### Why / context

- The pickers were already open natively but the recognizer only knew the
  Settings tab header and rejected any option marker row, so a single guarded
  send ended as an unknown receipt

### Evidence

- Retained original structural rows from Claude Code 2.1.296 at 80 columns
  (Haiku); HTTP owner-boundary regression covers both pickers and keeps
  autocomplete, dismissed-picker and historical-picker frames unknown

### Links

- <https://github.com/sympoies/nils-cli/issues/3062>
