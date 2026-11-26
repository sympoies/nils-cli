## 2026-11-27 - Check the console daemon helper exit status
<!-- devlog-id: 2026-11-27-issue-4104-daemon-helper-exit -->

### Result

- The console daemon regression test reaps its helper and verifies that the
  guard killed it with SIGKILL.

### Why / context

- Darwin can stop reporting an exiting process in a group probe while the
  individual PID still exists. The old assertion could fail after the guard had
  already killed the helper.

### Evidence

- The old assertion failed in all five controlled runs on macOS. The revised
  test passed 100 runs with the default temp root and 50 with a physical temp
  root.

### Links

- <https://github.com/sympoies/nils-cli/issues/4104>
