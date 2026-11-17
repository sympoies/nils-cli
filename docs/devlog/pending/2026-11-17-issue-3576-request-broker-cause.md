## 2026-11-17 - Retitle request-time broker faults preserve their cause
<!-- devlog-id: 2026-11-17-issue-3576-request-broker-cause -->

### Result

- Request-time account selection and credential resolution reuse the readiness
  cause classifier; broker rejection remains account-missing while start
  failures and unusable responses report broker unavailability and its
  configuration action. Same-key failed-request replay reconstructs the broker
  recovery details from the final durable provider attempt.

### Why / context

- Readiness classification alone did not cover broker faults occurring during a
  title request. Preserve the established provider-timeout recovery contract
  rather than changing timeout behavior or expanding the v2/v3 allowlists.
  Reconstructing replay from the diagnostic code alone lost the new broker
  variant; reuse retained attempt evidence without adding durable fields.
- The restart regression exposed a fixture port handoff race. Let each server
  bind its own ephemeral port and publish its endpoint; verify a unique startup
  identity so another fixture cannot satisfy health checks.

### Evidence

- The request-time regression failed first with retitle-account-missing for
  malformed broker output, then passed for automatic selection and
  configured-account resolution. The existing shared-deadline test passed
  unchanged. A request-level replay regression failed on lost recovery details,
  then passed across a serving-process restart for automatic and configured
  accounts while preserving a generic terminal fallback failure.
- A real foreign-server health check was accepted before the fixture repair
  and rejected afterward; restart ownership is distinct for every child.

### Links

- <https://github.com/sympoies/nils-cli/issues/3576>
