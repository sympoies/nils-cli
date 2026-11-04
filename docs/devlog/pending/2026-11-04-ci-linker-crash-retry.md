## 2026-11-04 - CI logs disk use and retries linker crashes once
<!-- devlog-id: 2026-11-04-ci-linker-crash-retry -->

### Result

- Linux CI check scripts log free disk space and target size at start, after
  the build and on failure.
- The build step is split from the test run and retried once only on an ld
  signal 7 or 9 crash; tests and compile errors are never retried.

### Why / context

- Intermittent linker bus errors cost full merge queue cycles and no log showed
  whether the disk was full.

### Evidence

- scripts/ci/tests/ci-build-guard.test.sh covers trigger and non-trigger cases.

### Links

- sympoies/nils-cli#3300
