## 2026-10-22 - Prevent initial prompt replay after delayed terminal rendering
<!-- devlog-id: 2026-10-22-issue-2798-startup-prompt-once -->

### Result

- Removed the startup paste replay based on unchanged pane digests; initial
  prompt bytes are sent once while uncertainty fences remain intact.

### Why / context

- A provider can accept input before rendering it, so matching screenshots
  cannot prove non-delivery.

### Evidence

- A delayed-render startup regression reproduced a doubled first turn before
  the fix and passes afterward; changed and unavailable observations plus
  accepted-but-failed paste are covered.

### Links

- <https://github.com/sympoies/nils-cli/issues/2798>
