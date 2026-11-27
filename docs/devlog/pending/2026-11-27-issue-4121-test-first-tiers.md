## 2026-11-27 - Align test-first evidence with delivery tiers
<!-- devlog-id: 2026-11-27-issue-4121-test-first-tiers -->

### Result

- Feature records can use acceptance criteria and passing interface tests bound
  to the delivery head. The recorder rejects compile errors, missing symbols,
  and timeouts as red evidence.

### Why / context

- A feature can be validated at its interface without manufacturing a pre-fix
  failure. Bug fixes still require a meaningful assertion failure or a waiver.

### Evidence

- The focused recorder fixtures and the fake-provider create, deliver,
  adoption, and dry-run matrix passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/4121>
