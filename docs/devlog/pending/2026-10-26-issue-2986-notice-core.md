## 2026-10-26 - Versioned display-only session notices
<!-- devlog-id: 2026-10-26-issue-2986-notice-core -->

### Result

- Added the public v1 notice grammar, language-neutral fixtures, shared codec,
  guarded native notice command and typed initial prompts; writers remain
  disabled by default.

### Why / context

- A display marker must preserve payload bytes without conferring authority or
  weakening input guards. Terminal continuation refuses until exact multiline
  composer verification is available; core delivery is separated from internal
  producer rollout.

### Evidence

- Boundary RED preceded implementation. Focused notice, modeled initial prompt
  and guarded-send regressions passed under bounded build resources; live
  provider and compaction readback remain pending.

### Links

- <https://github.com/sympoies/nils-cli/issues/2986>
