## 2026-11-06 - Expose exact owned image upload paths in history
<!-- devlog-id: 2026-11-06-issue-3378-history-upload-path -->

### Result

- Proven user-upload image descriptors and matching parts expose the same
  optional upload_path string; the history spec and fixture freeze its
  representation.

### Why / context

- Consumers need exact copyable upload paths after reload without
  reconstructing paths or broadening authenticated image-read authority.

### Evidence

- The focused upload/history fixture suite ran 24 tests before production
  edits: two expected missing-path failures and 22 passes, including ownership
  refusals.

### Links

- <https://github.com/sympoies/nils-cli/issues/3378>
