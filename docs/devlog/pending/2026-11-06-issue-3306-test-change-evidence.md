## 2026-11-06 - Require evidence for test changes with findings
<!-- devlog-id: 2026-11-06-issue-3306-test-change-evidence -->

### Result

- Test-kind changes with test-change findings require the configured bug
  evidence record.

### Why / context

- Test-only fixes can otherwise bypass the bug evidence gate.

### Evidence

- Focused forge-cli test-first policy integration suite.

### Links

- <https://github.com/sympoies/nils-cli/issues/3306>
