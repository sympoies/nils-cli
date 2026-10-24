## 2026-10-24 - Extract agent-session serve unit tests
<!-- devlog-id: 2026-10-24-issue-2886-serve-unit-tests -->

### Result

- Moved the inline serve::tests module into serve/tests.rs while preserving
  production code, test bodies, assertions, and child-module boundaries.

### Why / context

- Reduce serve.rs by more than 23,000 lines for the large-file ratchet without
  changing the tested behavior.

### Evidence

- Strict file-size audit passed; production source matched byte for byte, and
  the extracted tests matched the original after rustfmt and relative-path
  normalization.

### Links

- <https://github.com/sympoies/nils-cli/issues/2886>
