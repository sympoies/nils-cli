## 2026-11-04 - Share one batch runner for independent guarded-send cases
<!-- devlog-id: 2026-11-04-issue-3281-guarded-send-case-batches -->

### Result

- tests/guarded_send/cases.rs runs independent fixture cases in batches of at
  most 4 and names each failing case; eight loop tests use it (the
  runtime-command byte test, the Claude notice postclaim and launch-mount
  tests, the unknown-probe exit test, the Codex wrapped layout test, the
  rollback uncertainty test, the wrapped-rendering test and the notice summary
  test).

### Why / context

- Each case built its own fixture and ran the real binary one after another, so
  the tests took the sum of every case.

### Evidence

- Local, gate wrapper, one run each: 22.5 to 18.2 s, 18.7 to 14.8 s, 13.7 to
  4.1 s, 11.6 to 3.5 s, 10.9 to 4.7 s, 10.6 to 3.5 s, 10.5 to 5.6 s; repeat
  runs recorded in the pull request.

### Links

- <https://github.com/sympoies/nils-cli/issues/3281>
