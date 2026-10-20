## 2026-10-20 - Portable final investigation references
<!-- devlog-id: 2026-10-20-issue-2601-portable-final-references -->

### Result

- Final-resolution summaries display home-absolute investigation references
  portably while canonical evidence bytes, hashes and resolution history remain
  unchanged.
- URI-contained home components remain intact for the existing publication
  guard to refuse, preserving audit links rather than silently corrupting them.

### Why / context

- Immutable resolved evidence can contain local investigation references.
  Presentation must be portable at publication without replacing dispositions
  or weakening the provider path guard.

### Evidence

- Five export regressions, four stub publication regressions and five existing
  final-resolution tests pass; affected library Clippy and strict file-size
  audit pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/2601>
