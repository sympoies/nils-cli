## 2026-10-20 - Type session-only picker shortcuts through managed input
<!-- devlog-id: 2026-10-20-issue-2361-session-picker-keys -->

### Result

- agent-session send accepts single printable keys with mandatory whole-request
  idle admission and literal delivery.

### Why / context

- Provider model pickers can distinguish typed session-only shortcuts from
  pasted text and default-saving Enter; the CLI exposes transport while callers
  verify provider scope and settings.

### Evidence

- Five offline picker regressions verify synthetic Codex/Claude session
  changes, unchanged defaults and conversation identity, and refusal before any
  mixed-key input; a first-key-only guard mutation now fails.
- Two web contract tests retain HTTP rejection and WebSocket no-input behavior
  for single-character keys while named keys remain supported.

### Links

- <https://github.com/sympoies/nils-cli/issues/2361>
- <https://github.com/sympoies/nils-cli/pull/2694>
