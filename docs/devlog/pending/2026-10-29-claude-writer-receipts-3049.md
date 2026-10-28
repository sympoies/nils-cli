## 2026-10-29 - Add durable Claude account writer receipts
<!-- devlog-id: 2026-10-29-claude-writer-receipts-3049 -->

### Result

- Add exact-context per-store writer receipts and local snapshot publication
  with completion-last readback and legacy writer invalidation.

### Why / context

- Transport generation annotations and aggregate write success do not establish
  durable publication after partial writes or process interruption.

### Evidence

- Five meaningful synthetic regression failures were captured before production
  edits; focused auth integration checks validate interruption, readback
  replacement, restart replay, and cleanup.

### Links

- <https://github.com/sympoies/nils-cli/issues/3049>

### Follow-ups

- Keep consumer capability enablement and released producer/consumer acceptance
  separately coordinated; no live credential operation is included.
