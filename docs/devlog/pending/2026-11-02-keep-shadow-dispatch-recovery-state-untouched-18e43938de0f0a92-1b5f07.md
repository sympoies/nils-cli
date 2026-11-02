## 2026-11-02 - Keep shadow dispatch recovery state untouched
<!-- devlog-id: 2026-11-02-keep-shadow-dispatch-recovery-state-untouched-18e43938de0f0a92-1b5f07 -->

### Result

- Shadow dispatch skips conversation recovery state updates.

### Why / context

- Shadow evaluation must not create or consume live recovery state.

### Evidence

- Focused conversation recovery tests cover shadow and non-shadow behavior.

### Links

- <https://github.com/sympoies/nils-cli/issues/3232>
