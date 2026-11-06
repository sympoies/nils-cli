## 2026-11-06 - Star and resume sessions found by exact Claude id
<!-- devlog-id: 2026-11-06-issue-3395-exact-id-actions -->

### Result

- History star and resume resolve a Claude session found by exact id through
  the same direct-session lookup that lists and messages it, so rows beyond the
  catalog scan limit no longer advertise actions that fail.

### Why / context

- Star and resume used the bounded catalog while the list advertised both
  actions for direct-only rows. The direct guards (regular file, real project
  directory, configured source, readable cwd) are unchanged; resume re-reads
  metadata from the transcript.

### Evidence

- The exact-id catalog-limit fixture failed on star with not-found before the
  change and passes with star and resume; the symlink-swap guard now also
  rejects both actions.

### Links

- <https://github.com/sympoies/nils-cli/issues/3395>
