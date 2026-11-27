## 2026-11-28 - Paginate forge reverse references
<!-- devlog-id: 2026-11-28-forge-reverse-reference-pagination -->

### Result

- Reverse-reference queries traverse all GitHub timeline pages unless
  explicitly limited.

### Why / context

- Provider page size previously truncated results and signaled an incomplete
  query.

### Evidence

- Mocked multi-page and explicit-limit integration coverage.

### Links

- <https://github.com/sympoies/nils-cli/issues/4187>
