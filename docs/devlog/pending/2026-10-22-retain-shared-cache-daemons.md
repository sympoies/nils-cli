## 2026-10-22 - Retain shared build-cache daemons during session deletion
<!-- devlog-id: 2026-10-22-retain-shared-cache-daemons -->

### Result

- Linux deletion reports daemonized sccache servers as retained shared daemons
  while other marked survivors continue to block cleanup.

### Why / context

- Shared build caches may outlive the runtime whose environment they inherited.

### Evidence

- Focused deletion recovery regression tests; strict file-size audit;
  documentation hygiene audit.

### Links

- <https://github.com/sympoies/nils-cli/issues/2789>
