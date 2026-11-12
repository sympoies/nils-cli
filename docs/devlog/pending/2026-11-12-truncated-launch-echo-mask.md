## 2026-11-12 - agent-session: mask truncated launch-notice echoes in last_prompt
<!-- devlog-id: 2026-11-12-truncated-launch-echo-mask -->

### Result

- Claude's last-prompt and queued-input echoes of the bound launch notice are
  masked when they are a truncated prefix (newlines flattened, about 200
  characters plus an ellipsis), keyed to the launch binding
- Renamed the wrong-hash binding test to match its case and added an
  unbound-header case; refs #3588, #3492

### Why / context

- Launch notices are longer than the echo limit, so exact-equality masking let
  the notice header leak into last_prompt

### Evidence

- TODO
