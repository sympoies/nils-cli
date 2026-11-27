## 2026-11-28 - Make Claude notice attachment a host rule
<!-- devlog-id: 2026-11-28-issue-4186-claude-notice-host-rule -->

### Result

- Claude module attachment follows the host operator rule and missing
  attachment reasons appear in lifecycle and board views.

### Why / context

- Launches from different shells must agree on attachment while preserving
  notice delivery guards.

### Evidence

- Focused modeled launch and operator rules tests passed; the Linux-only
  restart regression is retained as a pre-fix test commit.

### Links

- <https://github.com/sympoies/nils-cli/issues/4186>
