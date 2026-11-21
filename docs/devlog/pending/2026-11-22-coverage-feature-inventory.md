## 2026-11-22 - Lock the full and scoped coverage inventories
<!-- devlog-id: 2026-11-22-coverage-feature-inventory -->

### Result

- Document release-default and selected all-feature coverage. Exercise both
  selection paths with an offline resolver-2 workspace.

### Why / context

- Selected packages can bring additional test targets and unify shared
  dependency features. The two coverage scores use different inputs.

### Evidence

- The linked Linux Markdown probe discovered 34 default and 46 all-feature
  tests. Locked resolution also covered macOS arm64.

### Links

- <https://github.com/sympoies/nils-cli/issues/2676#issuecomment-7069961668>
