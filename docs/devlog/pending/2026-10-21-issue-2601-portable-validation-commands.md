## 2026-10-21 - Portable validation command display preserves evidence identities
<!-- devlog-id: 2026-10-21-issue-2601-portable-validation-commands -->

### Result

- Final disposition summaries display home-absolute command references relative
  to the home variable while preserving canonical command identities and
  resolution bindings.

### Why / context

- An immutable command containing a log redirection could make generated
  publication text fail the local-path guard even when the supplied body was
  portable.

### Evidence

- Synthetic owner and publication regressions reproduced the refusal before
  repair; affected portability suites passed after repair, including URI
  refusal and byte-for-byte evidence preservation.

### Links

- <https://github.com/sympoies/nils-cli/issues/2601>
- <https://github.com/sympoies/nils-cli/pull/2706>
