## 2026-11-22 - Admit additive tests without fabricated test-first evidence
<!-- devlog-id: 2026-11-22-tests-only-pr-evidence -->

### Result

- Forge create and deliver admit additive coverage in dedicated test source
  directories and bind admission to the inspected provider head.

### Why / context

- Tests for already-correct behavior can be added honestly, while mixed changes
  and known test repairs still require evidence.

### Evidence

- The forge-cli test_first_kinds integration suite reproduced the old rejection
  and passed the revised admission matrix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3836>
