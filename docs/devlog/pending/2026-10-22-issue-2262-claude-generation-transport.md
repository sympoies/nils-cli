## 2026-10-22 - Preserve Claude credential generation context through remote pulls
<!-- devlog-id: 2026-10-22-issue-2262-claude-generation-transport -->

### Result

- Remote pulls validate optional token-free generation context before writes
  and report it beside actual per-account writer outcomes.

### Why / context

- Generation-aware session recovery needs snapshot identity without mistaking a
  successful materialization or partial write for publication.

### Evidence

- Three focused regressions failed before production edits; focused generation
  and legacy remote transport validation recorded in the PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/2262>

### Follow-ups

- Authority publication wiring, replay fences and the recovery controller
  remain separate slices.
