## 2026-11-04 - Run Linux CI test temp directories on tmpfs
<!-- devlog-id: 2026-11-04-issue-3281-ci-test-tmpfs -->

### Result

- On GitHub Linux runners the test step sets TMPDIR to a fresh directory under
  /dev/shm when it is writable with at least 2 GiB free, and logs the reason
  when it falls back.

### Why / context

- State writes fsync every file and directory, which dominates the state-heavy
  nils-agent-session tests on a real disk.

### Evidence

- Local, same host: one retention test 22 s on disk, 1.9 s on tmpfs;
  whole-package summed time fell about 27 percent. The CI JUnit number decides
  whether this stays.

### Links

- <https://github.com/sympoies/nils-cli/issues/3281>
