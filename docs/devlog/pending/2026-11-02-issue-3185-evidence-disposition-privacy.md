## 2026-11-02 - Keep generated validation dispositions private
<!-- devlog-id: 2026-11-02-issue-3185-evidence-disposition-privacy -->

### Result

- Generated PR dispositions now export command digests, status, scope, attempt
  and passing-resolution links; raw command and disposition text remain in
  immutable evidence.

### Why / context

- Freeform commands and disposition details can contain arbitrary local paths.
  A fixed typed projection replaces home-path rewriting without adding another
  sanitizer or changing record formats.

### Evidence

- The actual create backend captured leaked disposition text in the retained
  RED. The regression now passes through both create and deliver with unchanged
  evidence bytes; shared portability and existing publication-gate tests pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3185>
