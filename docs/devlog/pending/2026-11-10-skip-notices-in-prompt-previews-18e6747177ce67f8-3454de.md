## 2026-11-10 - Skip notices in prompt previews
<!-- devlog-id: 2026-11-10-skip-notices-in-prompt-previews-18e6747177ce67f8-3454de -->

### Result

- Session and History prompt previews skip classified notices when selecting
  user prompts.

### Why / context

- Runtime notices can otherwise replace the real prompt shown to users.

### Evidence

- Focused regression tests reproduce notice selection before the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3451>
