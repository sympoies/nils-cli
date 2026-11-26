## 2026-11-27 - Add code-scanning alert state transitions
<!-- devlog-id: 2026-11-27-code-scanning-alert-transitions -->

### Result

- forge-cli can dismiss and reopen GitHub code-scanning alerts using the
  selected write identity.

### Why / context

- Provide validated, scriptable alert state changes with a mutation-free dry
  run.

### Evidence

- Fake-provider integration tests cover request mapping and error handling.

### Links

- <https://github.com/sympoies/nils-cli/issues/2406>
