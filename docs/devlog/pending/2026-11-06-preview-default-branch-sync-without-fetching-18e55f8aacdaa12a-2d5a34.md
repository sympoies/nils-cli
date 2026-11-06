## 2026-11-06 - Preview default branch sync without fetching
<!-- devlog-id: 2026-11-06-preview-default-branch-sync-without-fetching-18e55f8aacdaa12a-2d5a34 -->

### Result

- Dry runs compare the cached remote-tracking ref and report that no fetch
  occurred.

### Why / context

- A preview should not update remote-tracking refs.

### Evidence

- A local bare-remote regression test reproduced the previous fetch.

### Links

- <https://github.com/sympoies/nils-cli/issues/3322>
