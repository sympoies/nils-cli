## 2026-11-05 - Report exhausted guarded identity observations as timeouts
<!-- devlog-id: 2026-11-05-issue-3329-identity-deadline -->

### Result

- Failed identity reads now report guard observation timeout when their
  observation budget has expired; identity comparisons and fail-closed input
  handling remain enforced.

### Why / context

- A subprocess could consume the remaining composer retry budget, but its
  failed read discarded the known deadline expiry and reported unavailable
  runtime identity.

### Evidence

- The original uncertain-wrapping test failed on baseline stress iteration 3/8
  under bounded burst CPU throttling. Fixed code passed 50 sequential and 50
  contended batch iterations with retries disabled; a focused regression
  distinguishes timeout, unavailable identity, and mismatch.

### Links

- <https://github.com/sympoies/nils-cli/issues/3329>
