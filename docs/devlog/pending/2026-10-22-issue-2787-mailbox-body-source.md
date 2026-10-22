## 2026-10-22 - Distinguish mailbox body sources from invalid contents
<!-- devlog-id: 2026-10-22-issue-2787-mailbox-body-source -->

### Result

- Body-file open, metadata, read, and source-type failures now return
  mailbox-body-source-invalid, including memfd procfs paths rejected by
  no-follow policy. Service-send preserves the source diagnostic and
  mailbox-body-invalid remains content-only.

### Why / context

- Callers need actionable source diagnostics without weakening no-follow
  regular-file handling or exposing paths and bodies.

### Evidence

- Focused CLI regression tests reproduced the source misclassification before
  the fix; both pass after the fix.
- Rust formatting, strict file-size audit, docs hygiene, and fragment checks
  passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2787>
