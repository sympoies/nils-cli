## 2026-10-21 - agent-session: session wrappers embed a version-stable executable path
<!-- devlog-id: 2026-10-21-stable-session-launch-executable -->

### Result

- Session launch wrappers (`broker_bin`, the app-server proxy helper,
  `AGENT_SESSION_BIN`) now embed a path that survives a package upgrade: the
  invocation path outside a versioned keg, else the package `opt/<package>`
  link, else the package `bin/` link. Symlinks are not resolved into the
  versioned directory.
- `agent-session diagnose` reports `runtime.launch_executable` and degrades
  health when only a versioned keg path exists, so the unsupported case is
  visible instead of silent.

### Why / context

- A pinned `Cellar/<package>/<version>/bin/agent-session` path is deleted by an
  upgrade: fixes never reached running sessions and the runtime-exit `broker
  stop` failed silently.

### Evidence

- Focused `stable_executable`, `diagnose` and held-launch wrapper tests; the
  wrapper test upgrades mid-run and asserts `broker stop` ran the new release.
- The resolver tests moved with the code into `stable_executable_tests.rs`; the
  PR body Test changes section lists them together with the new stable-link
  cases.

### Links

- sympoies/nils-cli#2672
