## 2026-11-06 - Resolve fresh history image selectors
<!-- devlog-id: 2026-11-06-issue-3418-fresh-history-images -->

### Result

- Fresh exact-session image reads use the guarded direct lookup.

### Why / context

- Catalog snapshots can lag discovery of a new transcript.

### Evidence

- Fixture reproduces the stale snapshot response and compares it with a
  refreshed catalog.

### Links

- <https://github.com/sympoies/nils-cli/issues/3418>
