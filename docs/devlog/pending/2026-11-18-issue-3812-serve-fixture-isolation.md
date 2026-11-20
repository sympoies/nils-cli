## 2026-11-18 - Isolate serve-spawning test fixtures
<!-- devlog-id: 2026-11-18-issue-3812-serve-fixture-isolation -->

### Result

- Agent Session integration fixtures start daemons with fixture-owned
  configuration, temporary storage, and tmux endpoints.

### Why / context

- Startup maintenance could otherwise resolve the test runner’s agent temporary
  root.

### Evidence

- The binary-replacement fixture accessed a synthetic outside root before the
  change. The same filesystem probe found zero accesses after the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3812>
