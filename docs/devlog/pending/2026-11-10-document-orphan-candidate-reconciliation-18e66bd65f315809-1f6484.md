## 2026-11-10 - Document orphan candidate reconciliation
<!-- devlog-id: 2026-11-10-document-orphan-candidate-reconciliation-18e66bd65f315809-1f6484 -->

### Result

- The release check points maintainers to reconciliation and avoids minting an
  App token when no candidate is available.

### Why / context

- An occupied branch must remain untouched while maintainers verify its
  history; in-range checks do not need candidate credentials.

### Evidence

- Release-check fixtures cover orphan refusal and publication planning.

### Links

- <https://github.com/sympoies/nils-cli/issues/3450>
