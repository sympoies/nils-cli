## 2026-11-27 - Generate test-change declarations from findings
<!-- devlog-id: 2026-11-27-generate-test-change-declarations-from-findings-18ebbb7fec5f5510-3077 -->

### Result

- The review preview emits copyable declaration rows and reports expected rows
  for malformed declarations.

### Why / context

- Authors need the same detected path and kind that review admission validates.

### Evidence

- Fake-provider review-handoff fixtures cover emission and malformed and
  mismatched declarations.

### Links

- <https://github.com/sympoies/nils-cli/issues/4128>
