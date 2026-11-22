## 2026-11-22 - Wake a fresh Codex mailbox before its first rollout
<!-- devlog-id: 2026-11-22-codex-first-mailbox-wake -->

### Result

- Use the exact idle thread rollout path to attest the first native wake.

### Why / context

- Fresh Codex threads have no on-disk baseline before the first turn, leaving
  their mailbox wake queued.

### Evidence

- The installed release refused before input on a fresh idle thread. A
  post-turn control received its wake.

### Links

- <https://github.com/sympoies/nils-cli/issues/3941>
