## 2026-11-14 - Claude permissions use exact live hook answers
<!-- devlog-id: 2026-11-14-claude-permission-hook-answers -->

### Result

- Claude permission answers now require a live PermissionRequest UUID and its
  exact projected PreToolUse call id; approve and deny are returned through the
  hook, and terminal approval inputs are refused.

### Why / context

- Keystrokes cannot bind an answer to a tool call and can reach a different
  dialog or composer. Uncorrelated, stale or ambiguous hooks stay conservative;
  reviewed version bounds and operator authority fence explicit answers.

### Evidence

- Synthetic regression recorded before production edits; 42 focused
  hook/session tests passed, including IPC, authorization, stale answer and
  timeout refresh cases.

### Links

- <https://github.com/sympoies/nils-cli/issues/3687>
