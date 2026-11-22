## 2026-11-22 - Gate code merge groups on instrumented macOS tests
<!-- devlog-id: 2026-11-22-issue-3932-macos-merge-queue -->

### Result

- Run both instrumented macOS partitions on ordinary code merge groups and
  require their success through the existing test context. Exempt changes
  confined to verified prose paths.

### Why / context

- The merge queue compiled macOS test targets but left execution to the release
  profile, which first caught regressions from an already-merged change.

### Evidence

- Queue dispatch fixtures cover the regression paths, shared and workflow
  inputs, a deletion, prose-only changes, and the release profile. Gate
  fixtures cover failed and absent partition results.

### Links

- <https://github.com/sympoies/nils-cli/issues/3932>
