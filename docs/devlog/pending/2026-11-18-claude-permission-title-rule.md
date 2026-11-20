## 2026-11-18 - Recognize titled Claude permission dialogs
<!-- devlog-id: 2026-11-18-claude-permission-title-rule -->

### Result

- Guarded permission keys admit a visible dialog followed by a terminal title
  rule.

### Why / context

- Titled panes otherwise fail the out-of-fence dialog visibility check.

### Evidence

- Synthetic guarded-send fixtures cover titled and untitled dialogs and stale
  or interrupted frames.

### Links

- <https://github.com/sympoies/nils-cli/issues/3806>
