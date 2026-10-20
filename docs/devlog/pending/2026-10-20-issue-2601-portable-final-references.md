## 2026-10-20 - Portable final investigation references
<!-- devlog-id: 2026-10-20-issue-2601-portable-final-references -->

### Result

- Final-resolution summaries display home-absolute investigation references
  portably while canonical evidence bytes, hashes and resolution history remain
  unchanged.

### Why / context

- Immutable resolved evidence can contain local investigation references.
  Presentation must be portable at publication without replacing dispositions
  or weakening the provider path guard.

### Evidence

- Four export regressions, three stub publication regressions and five existing
  final-resolution tests pass; affected library Clippy and strict file-size
  audit pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2601>
