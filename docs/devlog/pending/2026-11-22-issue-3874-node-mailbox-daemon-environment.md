## 2026-11-22 - Isolate Node mailbox acceptance daemons
<!-- devlog-id: 2026-11-22-issue-3874-node-mailbox-daemon-environment -->

### Result

- Build a fixture-only daemon environment and verify the exact state directory
  for both Node mailbox acceptance scripts.

### Why / context

- The remote fixture runs directly against installed binaries, so the daemon
  boundary belongs in the Node launchers.

### Evidence

- An execution trace saw the synthetic caller sentinel in all 13 baseline
  daemon launches and in none after the fix. Every guarded launch used the exact
  state directory, including both credential-collision launches.

### Links

- <https://github.com/sympoies/nils-cli/issues/3874>
