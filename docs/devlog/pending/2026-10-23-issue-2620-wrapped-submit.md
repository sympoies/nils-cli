## 2026-10-23 - Admit proven wrapped guarded pastes
<!-- devlog-id: 2026-10-23-issue-2620-wrapped-submit -->

### Result

- Guarded Claude sends can submit complete known ASCII pastes wrapped by pane
  width after empty-composer admission and cursor proof.

### Why / context

- Single-row submit observation refused every wrapped paste even when the
  cleanup matcher recognized its ownership.

### Evidence

- Failing-first narrow-width regression; all 95 guarded-send tests pass at
  final validation, including 32/44/70/80-column fixtures, draft collisions,
  and uncertain rendering refusals.
- Single-crate all-target/all-feature strict Clippy passes; build and test
  commands used a 6 GiB hard memory limit with swap disabled.

### Links

- <https://github.com/sympoies/nils-cli/issues/2620>
