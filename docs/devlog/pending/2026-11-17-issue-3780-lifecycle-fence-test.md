## 2026-11-17 - Isolate lifecycle fence test from concurrent forks
<!-- devlog-id: 2026-11-17-issue-3780-lifecycle-fence-test -->

### Result

- Run the shared-reader and exclusive-removal assertions in an isolated test
  process.

### Why / context

- A concurrent child can inherit a lock until exec and delay its release after
  the test drops its guard.

### Evidence

- The target failed once in 32 concurrent single-CPU baseline copies and passed
  in all 32 copies after isolation.

### Links

- <https://github.com/sympoies/nils-cli/issues/3780>
