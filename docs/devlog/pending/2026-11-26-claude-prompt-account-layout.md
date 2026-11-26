## 2026-11-26 - Align the Claude and Codex prompt-segment layouts
<!-- devlog-id: 2026-11-26-claude-prompt-account-layout -->

### Result

- Claude prompts put the matching account name before the usage windows and
  leave subscription plans to diagnostics.

### Why / context

- Use the same account-first layout for both providers without exposing
  credentials or full email addresses.

### Evidence

- Exact-output tests cover the weekly-only layout and existing time and
  timezone controls.

### Links

- <https://github.com/sympoies/nils-cli/issues/4049>
