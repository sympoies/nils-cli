## 2026-11-06 - Reuse console start trust configuration
<!-- devlog-id: 2026-11-06-issue-3428-console-trust-cache -->

### Result

- Console start requests reuse a process-local HTTP client and its system trust
  configuration.

### Why / context

- The system trust store was loaded for every routed start; roots refresh when
  the process restarts.

### Evidence

- The focused client build counter, console start unit suite and service route
  suite passed through the resource gate.

### Links

- <https://github.com/sympoies/nils-cli/issues/3428>
