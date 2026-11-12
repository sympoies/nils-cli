## 2026-11-12 - forge-cli test-changes declares deleted tests per test
<!-- devlog-id: 2026-11-12-test-changes-per-test-rows -->

### Result

- Removed tests now yield one deleted-test:<name> row each, including inline
  cfg(test) modules, so one declaration no longer covers sibling deletions.

### Why / context

- A file-level deleted-test row silently covered later deletions in the same
  file (#3581).

### Evidence

- Fixture: two deletions in one file with one declared row fails with
  test_changes_undeclared.

### Links

- sympoies/nils-cli#3581
