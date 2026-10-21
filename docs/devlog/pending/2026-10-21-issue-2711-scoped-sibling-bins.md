## 2026-10-21 - Build the sibling binaries a scoped run's tests resolve
<!-- devlog-id: 2026-10-21-issue-2711-scoped-sibling-bins -->

### Result

- The scoped executor builds, but does not test, the unselected packages whose
  binaries the selected packages' tests resolve, using the same scan local-fast
  uses.

### Why / context

- With forge-cli no longer escalating to FULL, a forge-cli-only merge group
  failed because its tests run the semantic-commit binary and a package-scoped
  run builds only the selected packages' own binaries.

### Evidence

- A consumer fixture test failed before the change (build lacked the sibling
  package) and passes after it; the real forge-cli test failed without the
  sibling build and passed with it.

### Links

- <https://github.com/sympoies/nils-cli/issues/2711>
