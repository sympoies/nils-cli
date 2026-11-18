## 2026-11-17 - Validate test change declarations before review admission
<!-- devlog-id: 2026-11-17-validate-test-change-declarations-before-review-admission-18e89ada3be16e21-3dfba6 -->

### Result

- Preview and review admission reject malformed declaration rows with the
  required syntax.

### Why / context

- The same declaration rules must apply before requesting review and when
  ledger prerequisites are absent.

### Evidence

- Synthetic integration tests reproduce both failures and verify the accepted
  list form.

### Links

- <https://github.com/sympoies/nils-cli/issues/3746>
