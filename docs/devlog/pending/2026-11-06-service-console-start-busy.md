## 2026-11-06 - Distinguish busy service console starts
<!-- devlog-id: 2026-11-06-service-console-start-busy -->

### Result

- Worker saturation reports a retryable refusal before creating an operation.

### Why / context

- Callers can retry a definite pre-start refusal without treating it as an
  unknown create outcome.

### Evidence

- A saturated-worker handler test reproduced the former unavailable code and
  passed with the busy response.

### Links

- <https://github.com/sympoies/nils-cli/issues/3391>
