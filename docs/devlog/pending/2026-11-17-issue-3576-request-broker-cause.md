## 2026-11-17 - Retitle request-time broker faults preserve their cause
<!-- devlog-id: 2026-11-17-issue-3576-request-broker-cause -->

### Result

- Request-time account selection and credential resolution reuse the readiness
  cause classifier; broker rejection remains account-missing while start
  failures and unusable responses report broker unavailability and its
  configuration action.

### Why / context

- Readiness classification alone did not cover broker faults occurring during a
  title request. Preserve the established provider-timeout recovery contract
  rather than changing timeout behavior or expanding the v2/v3 allowlists.

### Evidence

- The request-time regression failed first with retitle-account-missing for
  malformed broker output, then passed for automatic selection and
  configured-account resolution. The existing shared-deadline test passed
  unchanged.

### Links

- <https://github.com/sympoies/nils-cli/issues/3576>
