## 2026-11-24 - Display Claude Max 20x and pending reset grants
<!-- devlog-id: 2026-11-24-claude-ultra-reset-display -->

### Result

- The prompt segment and rate-limit diagnostics display Ultra for Max 20x and
  show pending granted resets from available usage status.

### Why / context

- Plan and grant labels come from stored login metadata and existing usage
  reads.

### Evidence

- Offline CLI fixtures reproduced both missing labels and passed after the
  change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3991>
