## 2026-11-18 - Reclaim abandoned gate slices with empty descendant units
<!-- devlog-id: 2026-11-18-issue-3813-gate-descendants -->

### Result

- Gate admission stops verified empty descendant units before stopping a dead
  holder’s slice and rechecks unit identity and subtree emptiness before each
  stop.

### Why / context

- An active empty scope can outlive a gate holder and block queued checks.

### Evidence

- Synthetic gate fixtures reproduced the blocked admission and verified the
  stop order and refusal cases.

### Links

- <https://github.com/sympoies/nils-cli/issues/3813>
