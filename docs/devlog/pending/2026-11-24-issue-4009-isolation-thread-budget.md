## 2026-11-24 - Keep the nextest thread limit in the isolation guard
<!-- devlog-id: 2026-11-24-issue-4009-isolation-thread-budget -->

### Result

- The guard preserves the configured nextest thread limit and uses the runner
  maximum when invoked directly. Hosted verification exercises both settings.

### Why / context

- Clearing the resource gate’s nextest setting let the poisoned lane start more
  tests than the host could schedule within the draft cleanup and prompt
  observation deadlines.

### Evidence

- On unmodified main with the affected fixtures pinned to one CPU, dropping the
  two-thread limit caused 19 failures. Preserving the limit let all 43 selected
  tests pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/4009>
