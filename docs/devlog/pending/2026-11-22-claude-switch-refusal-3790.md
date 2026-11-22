## 2026-11-22 - Disposable Claude switch refusal fixture
<!-- devlog-id: 2026-11-22-claude-switch-refusal-3790 -->

### Result

- Added an installed-runtime acceptance driver with a synthetic Claude session
  and account broker, typed HTTP refusal, metadata readback, and verified
  deletion.

### Why / context

- An installed consumer needs to observe the runtime account route refusing the
  switch without touching real accounts.

### Evidence

- Contract tests against agent-session 1.41.10 and 1.42.0 covered repeated
  refusal, unchanged runtime and account identity, and cleanup after consumer
  failure.

### Links

- <https://github.com/sympoies/nils-cli/issues/3790>
