## 2026-10-21 - local-fast fails clearly when a shared target is replaced
<!-- devlog-id: 2026-10-21-issue-2697-shared-target-replaced -->

### Result

- local-fast records the executables it built under the cargo target directory
  and re-checks them after the tests; a rewritten or removed executable now
  fails with 'shared target replaced' instead of a product test failure.

### Why / context

- Lanes sharing one CARGO_TARGET_DIR could overwrite the binaries under test
  mid-run, invalidating a long gate result.

### Evidence

- scripts/ci/tests/target-guard.test.py and
  scripts/ci/tests/local-fast-target-guard.test.py fail without the guard and
  pass with it.

### Links

- sympoies/nils-cli#2697
