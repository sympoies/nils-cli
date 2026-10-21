## 2026-10-22 - Isolate the test suite from the developer's real agent state
<!-- devlog-id: 2026-10-22-test-environment-isolation -->

### Result

- Test children now start from a cleared environment plus an allowlist with
  per-spawn homes, credential assertions never print text, and a
  poisoned-environment guard runs in the workspace lane.

### Why / context

- A developer-shell suite run inherited auth-sync hooks, agent homes and git
  config, copied real credentials into a fixture and wrote session state under
  the real agent home, while CI saw none of it.

### Evidence

- The guard over every leaking crate reports no sentinel change; a
  whole-workspace run with no home or config variables and a virtual X server
  passes apart from two time-window burst tests that flake only under heavy
  host load.

### Links

- <https://github.com/sympoies/nils-cli/issues/2755>

### Follow-ups

- Direct Command spawns outside the guard's selection still inherit the caller
  environment; add the target to the guard selection when a new leak class
  appears.
