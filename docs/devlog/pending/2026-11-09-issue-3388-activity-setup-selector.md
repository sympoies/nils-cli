## 2026-11-09 - Secure activity setup hook selection
<!-- devlog-id: 2026-11-09-issue-3388-activity-setup-selector -->

### Result

- Activity setup selects the hook from the shared trusted toolchain.

### Why / context

- Provider setup must not execute a hook found through PATH or a retired
  override.

### Evidence

- A poisoned PATH regression reproduced on main; focused activity setup tests
  pass with trusted roots.

### Links

- <https://github.com/sympoies/nils-cli/issues/3388>
