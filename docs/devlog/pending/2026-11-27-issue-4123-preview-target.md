## 2026-11-27 - Apply authoring target checks to all commit previews
<!-- devlog-id: 2026-11-27-issue-4123-preview-target -->

### Result

- Default-branch and cleanup previews share local authoring target selection.

### Why / context

- Previews previously accepted ambiguous push targets that real commits
  rejected.

### Evidence

- Synthetic fixtures compare real and preview refusals and verify local-only
  preview behavior.

### Links

- <https://github.com/sympoies/nils-cli/issues/4123>
