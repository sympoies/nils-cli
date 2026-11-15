## 2026-11-15 - Attribute temporary directory leaks to tests
<!-- devlog-id: 2026-11-15-attribute-temporary-directory-leaks-to-tests-18e822502193453f-3bd30c -->

### Result

- The local-fast leak probe records recursive names, types, timestamps and the
  owning nextest test when available.

### Why / context

- Disposable per-test temporary roots preserve attribution before cleanup
  without exposing file contents.

### Evidence

- Synthetic leak fixture and focused nextest probe.

### Links

- <https://github.com/sympoies/nils-cli/issues/2962>
