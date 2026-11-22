## 2026-11-23 - Interrupt managed Codex turns through the app-server
<!-- devlog-id: 2026-11-23-issue-3582-codex-turn-interrupt -->

### Result

- Add an owner-authenticated serve action that interrupts the exact current
  turn and returns the provider terminal outcome or an explicit idle result.

### Why / context

- Codex acknowledges the interrupt with an empty result. The matching turn
  completion or an exact provider readback distinguishes interruption from
  natural completion and failure.

### Evidence

- The fake app-server reproduces the baseline missing route and exercises exact
  turn targeting and the completion race.

### Links

- <https://github.com/sympoies/nils-cli/issues/3582#issuecomment-7085795683>
